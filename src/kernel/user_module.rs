// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. KernelState::LoadUserModule / FinishLoadingUserModule (kernel_state.cc), xeXexLoadImage /
// XexUnloadImage (xboxkrnl/xboxkrnl_modules.cc) and XexModule::SetupLibraryImports (cpu/xex_module.cc), Xenia Canary
// commit 451f915566d57436555c6eb4ee77bb4717eb01de.
//
//! XEX modules loaded after boot (XexLoadImage, XdfLoadXexFromCache): the image is mapped at its base, its imports
//! are bound (HLE kernel thunks, exports of loaded modules), it gets a loader entry, and a DLL's entry point runs as
//! DllMain(handle, DLL_PROCESS_ATTACH, 0) on the calling thread before the call returns.

use std::collections::HashMap;
use std::path::PathBuf;

use super::dispatcher::KernelDispatcher;
use super::guest_mem::GuestMem;
use super::modules::{ModuleExports, X_STATUS_INVALID_HANDLE, X_STATUS_SUCCESS};
use crate::loader::XexFile;
use crate::memory::MemoryBus;

pub const X_STATUS_NO_SUCH_FILE: u64 = 0xC000_000F;
const X_ERROR_INVALID_PARAMETER: u64 = 0x57;

/// A module entry point the calling thread runs before its kernel call returns (taken by the frame loop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleEntryCall {
    pub routine: u32,
    pub args: [u32; 3],
}

/// Host directory the title's modules are loaded from, and what a load adds for the frame loop.
#[derive(Debug, Default)]
pub struct UserModuleLoader {
    /// The folder of the title executable: modules named without a path are looked up there (LoadUserModule).
    pub title_dir: Option<PathBuf>,
    /// Addresses of the kernel variables the HLE kernel exports, by xboxkrnl ordinal (boot SetVariableMapping).
    pub kernel_variables: Vec<(u16, u32)>,
    /// Import thunks installed by loads since the frame loop last took them: address -> (library, ordinal).
    pub new_thunks: Vec<(u64, (String, u16))>,
    /// DllMain of a module loaded by the current call.
    pub pending_entry_call: Option<ModuleEntryCall>,
}

impl KernelDispatcher {
    /// KernelState::LoadUserModule + FinishLoadingUserModule: the module whose file name is the last component of
    /// `name`, from the title's folder. A loaded module (user or kernel) is returned as is; a new one is mapped,
    /// bound and registered, and its DllMain queued. Returns the loader entry (hmodule).
    pub fn load_user_module(&mut self, bus: &mut MemoryBus, name: &str) -> Result<u32, u64> {
        let file = name.rsplit(['\\', '/', ':']).next().unwrap_or(name);
        if let Some(module) = self.modules.find(file) {
            return Ok(module.hmodule);
        }
        let path = self.user_modules.title_dir.as_ref().map(|d| d.join(file)).ok_or(X_STATUS_NO_SUCH_FILE)?;
        let data = std::fs::read(&path).map_err(|_| X_STATUS_NO_SUCH_FILE)?;
        let load = || -> Result<(XexFile, Vec<u8>), String> {
            let xex = XexFile::parse(&data)?;
            let image = xex.decompress_image(&xex.decrypt_image_payload(&data)?)?;
            Ok((xex, image))
        };
        let (xex, image) = load().map_err(|e| {
            println!("    -> LoadUserModule('{}'): {}", path.display(), e);
            X_STATUS_NO_SUCH_FILE
        })?;
        let base = xex.image_base as u64;
        bus.map_image(base, xex.image_size as u64).map_err(|e| {
            println!("    -> LoadUserModule('{}'): {}", path.display(), e);
            X_STATUS_NO_SUCH_FILE
        })?;
        bus.load_bytes(base, &image[..image.len().min(xex.image_size as usize)]).map_err(|_| X_STATUS_NO_SUCH_FILE)?;

        // imports of other loaded modules, resolved before the records are rewritten
        let mut user_exports: HashMap<(String, u16), u32> = HashMap::new();
        let mut unresolved = Vec::new();
        for lib in xex.import_libraries.iter().filter(|l| !is_kernel_library(&l.name)) {
            for &record in &lib.import_table {
                let ordinal = (GuestMem::read_u32(bus, record as u64) & 0xFFFF) as u16;
                match self.modules.user_export(bus, &lib.name, ordinal) {
                    Some(address) => {
                        user_exports.insert((lib.name.clone(), ordinal), address);
                    }
                    None => unresolved.push(format!("{}!{}", lib.name, ordinal)),
                }
            }
        }
        unresolved.dedup();
        let thunks = xex
            .setup_imports(bus, &|lib, ordinal| user_exports.get(&(lib.to_string(), ordinal)).copied())
            .map_err(|_| X_STATUS_NO_SUCH_FILE)?;
        for (lib, ordinal, slot) in xex.variable_import_slots(bus) {
            if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                if let Some(&(_, address)) = self.user_modules.kernel_variables.iter().find(|(o, _)| *o == ordinal) {
                    GuestMem::write_u32(bus, slot as u64, address);
                }
            }
        }
        self.user_modules.new_thunks.extend(thunks);

        let exports = ModuleExports {
            image_base: xex.image_base,
            xex_table: xex.export_table,
            by_name: xex.exports_by_name.map_or(0, |offset| xex.image_base + offset),
        };
        let module_name = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
        let hmodule = self.modules.register_user_module(bus, module_name, &format!("game:\\{}", file), xex.image_size, xex.entry_point, xex.resources.clone(), exports);
        println!(
            "    -> LoadUserModule('{}'): 0x{:08X}+0x{:X}, entry 0x{:08X}, hmodule 0x{:08X}, {} module exports bound, unresolved: {:?}",
            path.display(), xex.image_base, xex.image_size, xex.entry_point, hmodule, user_exports.len(), unresolved
        );
        if xex.is_dll() && xex.entry_point != 0 {
            // DllMain(handle, DLL_PROCESS_ATTACH, 0 "always dynamic")
            let handle = self.modules.object_handle(bus, hmodule);
            self.user_modules.pending_entry_call = Some(ModuleEntryCall { routine: xex.entry_point, args: [handle, 1, 0] });
        }
        Ok(hmodule)
    }

    /// xeXexLoadImage(name, flags, min_version, hmodule_ptr): X_STATUS_SUCCESS with the loader entry, or
    /// X_STATUS_NO_SUCH_FILE with 0; each success adds to the module's load count.
    pub fn xex_load_image(&mut self, bus: &mut MemoryBus, name_ptr: u64, hmodule_ptr: u64) -> u64 {
        let hmodule_ptr = hmodule_ptr & 0xFFFF_FFFF;
        if hmodule_ptr == 0 {
            return X_ERROR_INVALID_PARAMETER;
        }
        let name = String::from_utf8_lossy(&GuestMem::read_cstr(bus, name_ptr & 0xFFFF_FFFF, 260)).into_owned();
        let (status, hmodule) = match self.load_user_module(bus, &name) {
            Ok(hmodule) => {
                self.modules.add_load_count(bus, hmodule, 1);
                (X_STATUS_SUCCESS, hmodule)
            }
            Err(status) => (status, 0),
        };
        GuestMem::write_u32(bus, hmodule_ptr, hmodule);
        status
    }

    /// XexUnloadImage(hmodule): a user module's load count drops (the image stays mapped: nothing is unloaded here,
    /// where Xenia releases the module at 0); kernel modules are left alone.
    pub fn xex_unload_image(&mut self, bus: &mut MemoryBus, hmodule: u64) -> u64 {
        let Some(module) = self.modules.by_hmodule(hmodule) else {
            return X_STATUS_INVALID_HANDLE;
        };
        if matches!(module.kind, super::modules::ModuleKind::User { .. }) {
            let hmodule = module.hmodule;
            self.modules.add_load_count(bus, hmodule, -1);
        }
        X_STATUS_SUCCESS
    }
}

fn is_kernel_library(name: &str) -> bool {
    name.eq_ignore_ascii_case("xboxkrnl.exe") || name.eq_ignore_ascii_case("xam.xex")
}
