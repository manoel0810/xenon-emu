// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Semantics of XexGetModuleHandle/XexGetModuleSection from
// src/xenia/kernel/xboxkrnl/xboxkrnl_modules.cc, XModule (xmodule.{h,cc}: X_LDR_DATA_TABLE_ENTRY, Matches,
// GetSection) and UserModule::GetSection (user_module.cc), Xenia Canary commit
// 451f915566d57436555c6eb4ee77bb4717eb01de.
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
const LDR_CHECKSUM: u64 = 0x48; // Xenia stores its kernel object handle here

pub const X_ERROR_SUCCESS: u64 = 0;
pub const X_ERROR_NOT_FOUND: u64 = 0x0000_0490;
pub const X_STATUS_SUCCESS: u64 = 0;
pub const X_STATUS_UNSUCCESSFUL: u64 = 0xC000_0001;
pub const X_STATUS_INVALID_HANDLE: u64 = 0xC000_0008;
pub const X_STATUS_NOT_FOUND: u64 = 0xC000_0225;

#[derive(Debug, Clone)]
pub enum ModuleKind {
    /// A loaded XEX (UserModule): sections come from its resource table.
    User { resources: Vec<XexResource> },
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
            ModuleKind::User { resources } => resources
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
        image_base: u32,
        image_size: u32,
        entry_point: u32,
        resources: Vec<XexResource>,
    ) -> u32 {
        let hmodule = self.alloc_ldr_entry(mem);
        let h = hmodule as u64;
        mem.write_u32(h + LDR_IMAGE_BASE, image_base);
        mem.write_u32(h + LDR_FULL_IMAGE_SIZE, image_size);
        mem.write_u32(h + LDR_ENTRY_POINT, entry_point);
        self.executable = Some(self.modules.len());
        self.modules.push(GuestModule { name: name.into(), path: path.into(), hmodule, kind: ModuleKind::User { resources } });
        hmodule
    }

    /// Registers an HLE kernel module (only the handle is meaningful).
    pub fn register_kernel(&mut self, mem: &mut dyn GuestMem, name: &str) -> u32 {
        let hmodule = self.alloc_ldr_entry(mem);
        self.modules.push(GuestModule { name: name.into(), path: name.into(), hmodule, kind: ModuleKind::Kernel });
        hmodule
    }

    fn by_hmodule(&self, hmodule: u64) -> Option<&GuestModule> {
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
        t.register_executable(mem, "dash", "game:\\dash.xex", 0x9200_0000, 0x0102_8000, 0x9219_6610,
            vec![res("dashuisk", 0x92C5_1700, 0x127F0), res("dashmain", 0x92E1_A380, 0xE4D), res("hubui", 0x92FC_F700, 0x2714)]);
        t.register_kernel(mem, "xboxkrnl.exe");
        t.register_kernel(mem, "xam.xex");
        t
    }

    const OUT: u64 = 0x5000;
    const NAME: u64 = 0x6000;

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
