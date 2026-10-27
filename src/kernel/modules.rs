// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Semantics of XexGetModuleHandle/XexGetModuleSection from
// src/xenia/kernel/xboxkrnl/xboxkrnl_modules.cc, XModule (xmodule.{h,cc}: X_LDR_DATA_TABLE_ENTRY, Matches,
// GetSection) and UserModule::GetSection (user_module.cc); XexGetProcedureAddress (xboxkrnl_modules.cc) and
// XexModule::GetProcAddress (cpu/xex_module.cc), Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de.
//
//! Loaded modules as the guest sees them: each module has an X_LDR_DATA_TABLE_ENTRY in guest memory whose
//! address is the HMODULE handed out by XexGetModuleHandle.

use super::guest_mem::GuestMem;
use crate::loader::xex::XexResource;

/// Guest area for the loader data entries (one X_LDR_DATA_TABLE_ENTRY per module). It sits after the per-thread
/// KPCR blocks (0x93100000 + 64 * 0x2000) and outside the dash image (which ends at 0x93028000).
pub const LDR_AREA: u64 = 0x931F_0000;
/// sizeof(X_LDR_DATA_TABLE_ENTRY), rounded up to 0x10.
const LDR_ENTRY_SIZE: u64 = 0x70;

// X_LDR_DATA_TABLE_ENTRY field offsets (xmodule.h)
const LDR_IMAGE_BASE: u64 = 0x1C;
const LDR_FULL_IMAGE_SIZE: u64 = 0x38;
const LDR_ENTRY_POINT: u64 = 0x3C;
const LDR_LOAD_COUNT: u64 = 0x40; // be u16
const LDR_CHECKSUM: u64 = 0x48; // Xenia stores its kernel object handle here

pub const X_ERROR_SUCCESS: u64 = 0;
pub const X_ERROR_NOT_FOUND: u64 = 0x0000_0490;
pub const X_STATUS_SUCCESS: u64 = 0;
pub const X_STATUS_UNSUCCESSFUL: u64 = 0xC000_0001;
pub const X_STATUS_INVALID_HANDLE: u64 = 0xC000_0008;
pub const X_STATUS_NOT_FOUND: u64 = 0xC000_0225;
pub const X_STATUS_DRIVER_ENTRYPOINT_NOT_FOUND: u64 = 0xC000_0263;

/// Where a loaded XEX publishes its exports (both live in its image).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModuleExports {
    pub image_base: u32,
    /// Guest address of the xex2_export_table (0 = none): exports by ordinal.
    pub xex_table: u32,
    /// Guest address of the PE export directory (XEX_HEADER_EXPORTS_BY_NAME, 0 = none): exports by name.
    pub by_name: u32,
}

impl ModuleExports {
    /// XexModule::GetProcAddress(ordinal): the xex2_export_table when there is one (ordinal - base indexes
    /// ordOffset, plus imagebaseaddr << 16), otherwise the PE directory's function table indexed by the ordinal.
    pub fn by_ordinal(&self, mem: &dyn GuestMem, ordinal: u32) -> Option<u32> {
        if self.xex_table != 0 {
            let t = self.xex_table as u64;
            let (image_base, count, base) = (mem.read_u32(t + 0x20), mem.read_u32(t + 0x24), mem.read_u32(t + 0x28));
            let index = ordinal.wrapping_sub(base);
            return (index < count).then(|| (image_base << 16).wrapping_add(mem.read_u32(t + 0x2C + 4 * index as u64)));
        }
        let dir = self.pe_directory(mem)?;
        (ordinal < dir.functions).then(|| self.image_base.wrapping_add(le32(mem, dir.function_table + 4 * ordinal as u64)))
    }

    /// XexModule::GetProcAddress(name): exact match in the PE directory's name table; the name's ordinal indexes
    /// the function table.
    pub fn by_name(&self, mem: &dyn GuestMem, name: &[u8]) -> Option<u32> {
        let dir = self.pe_directory(mem)?;
        (0..dir.names as u64).find_map(|i| {
            let name_addr = dir.base + le32(mem, dir.name_table + 4 * i) as u64;
            let ordinal = le16(mem, dir.ordinal_table + 2 * i) as u64;
            (mem.read_cstr(name_addr, 256) == name).then(|| self.image_base.wrapping_add(le32(mem, dir.function_table + 4 * ordinal)))
        })
    }

    /// X_IMAGE_EXPORT_DIRECTORY: little-endian, its table offsets relative to the directory.
    fn pe_directory(&self, mem: &dyn GuestMem) -> Option<PeExportDirectory> {
        if self.by_name == 0 {
            return None;
        }
        let base = self.by_name as u64;
        Some(PeExportDirectory {
            base,
            functions: le32(mem, base + 0x14),
            names: le32(mem, base + 0x18),
            function_table: base + le32(mem, base + 0x1C) as u64,
            name_table: base + le32(mem, base + 0x20) as u64,
            ordinal_table: base + le32(mem, base + 0x24) as u64,
        })
    }
}

struct PeExportDirectory {
    base: u64,
    functions: u32,
    names: u32,
    function_table: u64,
    name_table: u64,
    ordinal_table: u64,
}

fn le32(mem: &dyn GuestMem, addr: u64) -> u32 {
    mem.read_u32(addr).swap_bytes()
}

fn le16(mem: &dyn GuestMem, addr: u64) -> u16 {
    mem.read_u16(addr).swap_bytes()
}

#[derive(Debug, Clone)]
pub enum ModuleKind {
    /// A loaded XEX (UserModule): sections come from its resource table, procedures from its exports.
    User { resources: Vec<XexResource>, exports: ModuleExports },
    /// An HLE kernel module (xboxkrnl.exe, xam.xex): XModule::GetSection is unsuccessful.
    Kernel,
}

#[derive(Debug, Clone)]
pub struct GuestModule {
    pub name: String,
    pub path: String,
    /// Guest address of the module's X_LDR_DATA_TABLE_ENTRY.
    pub hmodule: u32,
    pub kind: ModuleKind,
}

impl GuestModule {
    /// XModule::Matches: case-insensitive match against the file name of the path, the name or the full path.
    fn matches(&self, name: &str) -> bool {
        let file = self.path.rsplit(['\\', '/', ':']).next().unwrap_or(&self.path);
        file.eq_ignore_ascii_case(name) || self.name.eq_ignore_ascii_case(name) || self.path.eq_ignore_ascii_case(name)
    }

    /// UserModule::GetSection / XModule::GetSection.
    fn section(&self, name: &[u8]) -> Result<(u32, u32), u64> {
        match &self.kind {
            ModuleKind::Kernel => Err(X_STATUS_UNSUCCESSFUL),
            ModuleKind::User { resources, .. } => resources
                .iter()
                .find(|r| r.name_bytes() == name)
                .map(|r| (r.address, r.size))
                .ok_or(X_STATUS_NOT_FOUND),
        }
    }
}

impl ModuleTable {
    /// KernelState::GetModule(name, user_only = true): the handle of a loaded XEX (not an HLE kernel module) whose
    /// name is `name` or `name`.xex.
    pub fn user_module_handle(&self, name: &str) -> Option<u32> {
        self.modules
            .iter()
            .find(|m| matches!(m.kind, ModuleKind::User { .. }) && (m.matches(name) || m.matches(&format!("{}.xex", name))))
            .map(|m| m.hmodule)
    }
}

#[derive(Debug, Clone, Default)]
pub struct ModuleTable {
    pub modules: Vec<GuestModule>,
    /// Index of the title executable (GetExecutableModule).
    pub executable: Option<usize>,
}

impl ModuleTable {
    fn alloc_ldr_entry(&mut self, mem: &mut dyn GuestMem) -> u32 {
        let idx = self.modules.len() as u64;
        let addr = LDR_AREA + idx * LDR_ENTRY_SIZE;
        for off in (0..LDR_ENTRY_SIZE).step_by(4) {
            mem.write_u32(addr + off, 0);
        }
        // A stable non-zero stand-in for the kernel object handle Xenia keeps in `checksum`.
        mem.write_u32(addr + LDR_CHECKSUM, 0xF800_0000 + 4 * idx as u32);
        addr as u32
    }

    /// Registers the title executable and fills its loader entry like UserModule::LoadFromMemory
    /// (dll_base = 0, image_base, full_image_size, entry_point; xex_header_base stays 0: the XEX header is not
    /// mapped into guest memory here).
    pub fn register_executable(
        &mut self,
        mem: &mut dyn GuestMem,
        name: &str,
        path: &str,
        image_size: u32,
        entry_point: u32,
        resources: Vec<XexResource>,
        exports: ModuleExports,
    ) -> u32 {
        self.executable = Some(self.modules.len());
        self.register_user_module(mem, name, path, image_size, entry_point, resources, exports)
    }

    /// Registers a loaded XEX (KernelState::LoadUserModule) and fills its loader entry like the executable's.
    pub fn register_user_module(
        &mut self,
        mem: &mut dyn GuestMem,
        name: &str,
        path: &str,
        image_size: u32,
        entry_point: u32,
        resources: Vec<XexResource>,
        exports: ModuleExports,
    ) -> u32 {
        let hmodule = self.alloc_ldr_entry(mem);
        let h = hmodule as u64;
        mem.write_u32(h + LDR_IMAGE_BASE, exports.image_base);
        mem.write_u32(h + LDR_FULL_IMAGE_SIZE, image_size);
        mem.write_u32(h + LDR_ENTRY_POINT, entry_point);
        self.modules.push(GuestModule { name: name.into(), path: path.into(), hmodule, kind: ModuleKind::User { resources, exports } });
        hmodule
    }

    /// KernelState::GetModule(name): any loaded module, user or kernel, by XModule::Matches.
    pub fn find(&self, name: &str) -> Option<&GuestModule> {
        self.modules.iter().find(|m| m.matches(name))
    }

    /// The kernel object handle Xenia keeps in the loader entry's checksum (XModule::handle()).
    pub fn object_handle(&self, mem: &dyn GuestMem, hmodule: u32) -> u32 {
        mem.read_u32(hmodule as u64 + LDR_CHECKSUM)
    }

    /// X_LDR_DATA_TABLE_ENTRY.load_count += delta (not below 0); returns the new count.
    pub fn add_load_count(&self, mem: &mut dyn GuestMem, hmodule: u32, delta: i32) -> u16 {
        let addr = hmodule as u64 + LDR_LOAD_COUNT;
        let count = (mem.read_u16(addr) as i32 + delta).max(0) as u16;
        mem.write_u16(addr, count);
        count
    }

    /// The export `ordinal` of the loaded XEX whose path or name is `library` (an import of another module).
    pub fn user_export(&self, mem: &dyn GuestMem, library: &str, ordinal: u16) -> Option<u32> {
        match &self.find(library)?.kind {
            ModuleKind::User { exports, .. } => exports.by_ordinal(mem, ordinal as u32).filter(|&a| a != 0),
            ModuleKind::Kernel => None,
        }
    }

    /// Registers an HLE kernel module (only the handle is meaningful).
    pub fn register_kernel(&mut self, mem: &mut dyn GuestMem, name: &str) -> u32 {
        let hmodule = self.alloc_ldr_entry(mem);
        self.modules.push(GuestModule { name: name.into(), path: name.into(), hmodule, kind: ModuleKind::Kernel });
        hmodule
    }

    pub fn by_hmodule(&self, hmodule: u64) -> Option<&GuestModule> {
        let h = hmodule & 0xFFFF_FFFF;
        if h == 0 {
            return None;
        }
        self.modules.iter().find(|m| m.hmodule as u64 == h)
    }

    /// XexGetModuleHandle(module_name, hmodule_ptr): NULL/empty name = the executable.
    pub fn xex_get_module_handle(&self, mem: &mut dyn GuestMem, name_ptr: u64, hmodule_ptr: u64) -> u64 {
        let name_ptr = name_ptr & 0xFFFF_FFFF;
        let name = if name_ptr == 0 { String::new() } else { String::from_utf8_lossy(&mem.read_cstr(name_ptr, 260)).into_owned() };
        let module = if name.is_empty() {
            self.executable.map(|i| &self.modules[i])
        } else {
            self.modules.iter().find(|m| m.matches(&name))
        };
        let hmodule_ptr = hmodule_ptr & 0xFFFF_FFFF;
        match module {
            Some(m) => {
                if hmodule_ptr != 0 {
                    mem.write_u32(hmodule_ptr, m.hmodule);
                }
                X_ERROR_SUCCESS
            }
            None => {
                if hmodule_ptr != 0 {
                    mem.write_u32(hmodule_ptr, 0);
                }
                X_ERROR_NOT_FOUND
            }
        }
    }

    /// XexGetProcedureAddress(hmodule, ordinal, out_ptr): a value with any of its upper 16 bits set is a pointer to
    /// the procedure's name; hmodule 0 is the executable. HLE kernel modules have no guest exports here (Xenia
    /// resolves their kernel export trampolines), so their lookups fail like a missing export.
    pub fn xex_get_procedure_address(&self, mem: &mut dyn GuestMem, hmodule: u64, ordinal: u64, out_ptr: u64) -> u64 {
        let module = if hmodule & 0xFFFF_FFFF == 0 { self.executable.map(|i| &self.modules[i]) } else { self.by_hmodule(hmodule) };
        let Some(module) = module else {
            return X_STATUS_INVALID_HANDLE;
        };
        let ordinal = ordinal as u32;
        let address = match &module.kind {
            ModuleKind::User { exports, .. } if ordinal & 0xFFFF_0000 != 0 => exports.by_name(mem, &mem.read_cstr(ordinal as u64, 256)),
            ModuleKind::User { exports, .. } => exports.by_ordinal(mem, ordinal),
            ModuleKind::Kernel => None,
        }
        .filter(|&a| a != 0);
        let out_ptr = out_ptr & 0xFFFF_FFFF;
        if out_ptr != 0 {
            mem.write_u32(out_ptr, address.unwrap_or(0));
        }
        if address.is_some() { X_STATUS_SUCCESS } else { X_STATUS_DRIVER_ENTRYPOINT_NOT_FOUND }
    }

    /// XexGetModuleSection(hmodule, name, data_ptr, size_ptr).
    pub fn xex_get_module_section(&self, mem: &mut dyn GuestMem, hmodule: u64, name_ptr: u64, data_ptr: u64, size_ptr: u64) -> u64 {
        let Some(module) = self.by_hmodule(hmodule) else {
            return X_STATUS_INVALID_HANDLE;
        };
        let name = mem.read_cstr(name_ptr & 0xFFFF_FFFF, 260);
        match module.section(&name) {
            Ok((data, size)) => {
                mem.write_u32(data_ptr & 0xFFFF_FFFF, data);
                mem.write_u32(size_ptr & 0xFFFF_FFFF, size);
                X_STATUS_SUCCESS
            }
            Err(status) => status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;

    fn res(name: &str, address: u32, size: u32) -> XexResource {
        let mut n = [0u8; 8];
        n[..name.len()].copy_from_slice(name.as_bytes());
        XexResource { name: n, address, size }
    }

    fn table(mem: &mut FakeMem) -> ModuleTable {
        let mut t = ModuleTable::default();
        t.register_executable(mem, "dash", "game:\\dash.xex", 0x0102_8000, 0x9219_6610,
            vec![res("dashuisk", 0x92C5_1700, 0x127F0), res("dashmain", 0x92E1_A380, 0xE4D), res("hubui", 0x92FC_F700, 0x2714)],
            ModuleExports { image_base: 0x9200_0000, ..Default::default() });
        t.register_kernel(mem, "xboxkrnl.exe");
        t.register_kernel(mem, "xam.xex");
        t
    }

    const OUT: u64 = 0x5000;
    const NAME: u64 = 0x6000;
    /// A name pointer XexGetProcedureAddress tells from an ordinal (upper 16 bits set).
    const PROC_NAME: u64 = 0x4100_6000;

    #[test]
    fn module_handle_of_executable_and_kernel_modules() {
        let mut m = FakeMem::default();
        let t = table(&mut m);
        let dash = t.modules[0].hmodule;
        assert_eq!(dash as u64, LDR_AREA);
        // GetModuleHandle(NULL)
        assert_eq!(t.xex_get_module_handle(&mut m, 0, OUT), X_ERROR_SUCCESS);
        assert_eq!(m.read_u32(OUT), dash);
        // loader entry filled like UserModule
        assert_eq!(m.read_u32(dash as u64 + LDR_IMAGE_BASE), 0x9200_0000);
        assert_eq!(m.read_u32(dash as u64 + LDR_FULL_IMAGE_SIZE), 0x0102_8000);
        assert_eq!(m.read_u32(dash as u64 + LDR_ENTRY_POINT), 0x9219_6610);
        assert_ne!(m.read_u32(dash as u64 + LDR_CHECKSUM), 0);
        // by name, case-insensitive, file name of the path or module name
        for (name, idx) in [("xam.xex", 2), ("XBOXKRNL.EXE", 1), ("dash.xex", 0), ("dash", 0)] {
            m.put_str(NAME, name);
            assert_eq!(t.xex_get_module_handle(&mut m, NAME, OUT), X_ERROR_SUCCESS, "{name}");
            assert_eq!(m.read_u32(OUT), t.modules[idx].hmodule, "{name}");
        }
    }

    #[test]
    fn module_handle_not_found_clears_output() {
        let mut m = FakeMem::default();
        let t = table(&mut m);
        m.write_u32(OUT, 0xDEAD_BEEF);
        m.put_str(NAME, "xbdm.xex");
        assert_eq!(t.xex_get_module_handle(&mut m, NAME, OUT), X_ERROR_NOT_FOUND);
        assert_eq!(m.read_u32(OUT), 0);
    }

    #[test]
    fn module_section_lookup() {
        let mut m = FakeMem::default();
        let t = table(&mut m);
        let dash = t.modules[0].hmodule as u64;
        // 8-character name without terminator in the table (the dash's skin resource)
        m.put_str(NAME, "dashuisk");
        assert_eq!(t.xex_get_module_section(&mut m, dash, NAME, OUT, OUT + 4), X_STATUS_SUCCESS);
        assert_eq!((m.read_u32(OUT), m.read_u32(OUT + 4)), (0x92C5_1700, 0x127F0));
        // shorter, NUL-padded name
        m.put_str(NAME, "hubui");
        assert_eq!(t.xex_get_module_section(&mut m, dash, NAME, OUT, OUT + 4), X_STATUS_SUCCESS);
        assert_eq!(m.read_u32(OUT), 0x92FC_F700);
        // prefix of a longer name and case differences do not match
        for name in ["hub", "dashuiskin", "DASHUISK"] {
            m.write_u32(OUT, 0x1234);
            m.put_str(NAME, name);
            assert_eq!(t.xex_get_module_section(&mut m, dash, NAME, OUT, OUT + 4), X_STATUS_NOT_FOUND, "{name}");
            assert_eq!(m.read_u32(OUT), 0x1234, "{name}: outputs untouched on failure");
        }
    }

    /// dashnui.xex-like exports: an xex2_export_table at 0x9E7FEEAC (imagebaseaddr 0x9E51, ordinals 1..=2) and a PE
    /// directory at 0x9E84E000 (little-endian, offsets relative to it) naming ordinal 1 "NuiShutdown".
    fn nui_exports(m: &mut FakeMem) -> ModuleExports {
        let t = 0x9E7F_EEAC;
        m.write_u32(t + 0x20, 0x9E51);
        m.write_u32(t + 0x24, 2);
        m.write_u32(t + 0x28, 1);
        m.write_u32(t + 0x2C, 0x000E_9728);
        m.write_u32(t + 0x30, 0x000E_8D38);
        let d = 0x9E84_E000u64;
        let le = |m: &mut FakeMem, a: u64, v: u32| m.write_u32(a, v.swap_bytes());
        le(m, d + 0x14, 2); // NumberOfFunctions
        le(m, d + 0x18, 1); // NumberOfNames
        le(m, d + 0x1C, 0x40); // AddressOfFunctions
        le(m, d + 0x20, 0x50); // AddressOfNames
        le(m, d + 0x24, 0x58); // AddressOfNameOrdinals
        le(m, d + 0x44, 0x0027_C238); // function_table[1]
        le(m, d + 0x50, 0x60); // name 0 at +0x60
        m.write_u16(d + 0x58, 1u16.swap_bytes()); // ordinal of name 0
        m.put_str(d + 0x60, "NuiShutdown");
        ModuleExports { image_base: 0x9E51_0000, xex_table: t as u32, by_name: d as u32 }
    }

    #[test]
    fn exports_by_ordinal_and_name_follow_xex_module() {
        let mut m = FakeMem::default();
        let mut t = table(&mut m);
        let e = nui_exports(&mut m);
        assert_eq!((e.by_ordinal(&m, 1), e.by_ordinal(&m, 2)), (Some(0x9E5F_9728), Some(0x9E5F_8D38)));
        assert_eq!((e.by_ordinal(&m, 0), e.by_ordinal(&m, 3)), (None, None), "ordinal - base outside the table");
        assert_eq!(e.by_name(&m, b"NuiShutdown"), Some(0x9E78_C238));
        assert_eq!((e.by_name(&m, b"nuishutdown"), e.by_name(&m, b"NuiShutdow")), (None, None), "exact match only");
        // without the xex2 table, ordinals index the PE function table directly
        assert_eq!(ModuleExports { xex_table: 0, ..e }.by_ordinal(&m, 1), Some(0x9E78_C238));

        let nui = t.register_user_module(&mut m, "dashnui", "game:\\dashnui.xex", 0x34_4000, 0x9E78_D458, Vec::new(), e);
        assert_eq!(t.user_export(&m, "dashnui.xex", 2), Some(0x9E5F_8D38));
        assert_eq!(t.user_export(&m, "xam.xex", 2), None, "kernel modules export nothing here");
        // XexGetProcedureAddress: by name (upper bits set), by ordinal, missing, bad handle, kernel module
        m.put_str(PROC_NAME, "NuiShutdown");
        assert_eq!(t.xex_get_procedure_address(&mut m, nui as u64, PROC_NAME, OUT), X_STATUS_SUCCESS);
        assert_eq!(m.read_u32(OUT), 0x9E78_C238);
        assert_eq!(t.xex_get_procedure_address(&mut m, nui as u64, 1, OUT), X_STATUS_SUCCESS);
        assert_eq!(m.read_u32(OUT), 0x9E5F_9728);
        m.put_str(PROC_NAME, "NuipInitialize");
        assert_eq!(t.xex_get_procedure_address(&mut m, nui as u64, PROC_NAME, OUT), X_STATUS_DRIVER_ENTRYPOINT_NOT_FOUND);
        assert_eq!(m.read_u32(OUT), 0);
        assert_eq!(t.xex_get_procedure_address(&mut m, 0x1234, 1, OUT), X_STATUS_INVALID_HANDLE);
        let xam = t.modules[2].hmodule as u64;
        assert_eq!(t.xex_get_procedure_address(&mut m, xam, 1, OUT), X_STATUS_DRIVER_ENTRYPOINT_NOT_FOUND);
        // hmodule 0 is the executable (no exports in this table)
        assert_eq!(t.xex_get_procedure_address(&mut m, 0, 1, OUT), X_STATUS_DRIVER_ENTRYPOINT_NOT_FOUND);
        // loader entry and load count
        assert_eq!(m.read_u32(nui as u64 + LDR_IMAGE_BASE), 0x9E51_0000);
        assert_eq!((t.add_load_count(&mut m, nui, 1), t.add_load_count(&mut m, nui, 1), t.add_load_count(&mut m, nui, -1)), (1, 2, 1));
        assert_eq!(m.read_u16(nui as u64 + LDR_LOAD_COUNT), 1);
        assert_eq!(t.find("DASHNUI.XEX").map(|m| m.hmodule), Some(nui));
        assert_eq!(t.executable, Some(0), "registering a module does not change the executable");
    }

    #[test]
    fn module_section_bad_handle_and_kernel_module() {
        let mut m = FakeMem::default();
        let t = table(&mut m);
        m.put_str(NAME, "dashuisk");
        assert_eq!(t.xex_get_module_section(&mut m, 0, NAME, OUT, OUT + 4), X_STATUS_INVALID_HANDLE);
        assert_eq!(t.xex_get_module_section(&mut m, 0x9200_0000, NAME, OUT, OUT + 4), X_STATUS_INVALID_HANDLE);
        let xam = t.modules[2].hmodule as u64;
        assert_eq!(t.xex_get_module_section(&mut m, xam, NAME, OUT, OUT + 4), X_STATUS_UNSUCCESSFUL);
    }
}
