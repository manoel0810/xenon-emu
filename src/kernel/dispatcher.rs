//! KernelDispatcher: HLE kernel state (threads, events, VFS, video) and the export dispatch entry point.

use std::collections::HashMap;
use crate::memory::MemoryBus;
use super::exports::{get_xboxkrnl_name, get_xam_name};

pub struct KernelFunction {
    pub name: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone)]
pub struct PendingThread {
    pub tid: u32,
    pub handle: u32,
    pub start_routine: u64,
    pub start_context: u64,
    pub stack_size: u32,
    pub creation_flags: u32,
}

#[derive(Debug, Clone)]
pub struct KernelEvent {
    pub handle: u32,
    pub event_type: u32, // Notification (0) or Synchronization (1)
    pub signaled: bool,
    pub guest_ptr: u64,
}

#[derive(Debug, Clone)]
pub struct VfsFileHandle {
    pub handle: u32,
    pub name: String,
    pub data: Vec<u8>,
    pub position: usize,
    pub is_directory: bool,
}

pub struct KernelDispatcher {
    pub next_thread_id: u32,
    pub next_handle: u32,
    pub tls_slots: HashMap<u32, u64>,
    pub next_tls_index: u32,
    /// Bump pointer for guest virtual allocations (NtAllocateVirtualMemory + HLE helper buffers).
    /// Lives in the bus virtual-alloc window (0x40000000..0x8FFFFFFF), which has private backing pages.
    pub valloc_bump: u64,
    /// Bump pointer for physical allocations (MmAllocatePhysicalMemoryEx: ring buffer, framebuffers, GPU data).
    /// These keep `addr & 0x1FFFFFFF` as their physical address, so they must not overlap the dash image (phys 0x12000000).
    pub phys_bump: u64,
    pub xam_app_registered: bool,
    /// Guest address of the cached `{ data*, size }` struct returned by XamGetOnlineSchema (0 = not built yet)
    pub online_schema_ptr: u64,
    /// xorshift64 state for XeCryptRandom
    pub rng_state: u64,
    pub xam_app_id: u32,
    pub signin_state: u32,
    pub perf_counter: u64,
    pub ring_buffer_ptr: u64,
    pub ring_buffer_size: u32,
    pub swap_count: u64,
    pub last_swap_frontbuffer: u64,
    pub rptr_writeback_ptr: u64,
    pub graphics_interrupt_callback: u64,
    pub graphics_interrupt_context: u64,
    pub frontbuffer_ptr: u64,
    pub backbuffer_ptr: u64,
    pub gpu_identifier_ptr: u64,
    pub gamepad_state: crate::kernel::input::XInputGamepad,
    pub gamepad_packet_number: u32,
    pub pending_threads: Vec<PendingThread>,
    pub events: HashMap<u32, KernelEvent>,
    pub event_by_ptr: HashMap<u64, u32>,
    pub vfs_files: HashMap<String, Vec<u8>>,
    /// Loaded modules and their loader data entries (XexGetModuleHandle / XexGetModuleSection).
    pub modules: super::modules::ModuleTable,
    pub vfs_handles: HashMap<u32, VfsFileHandle>,
}

impl KernelDispatcher {
    pub fn new() -> Self {
        Self {
            next_thread_id: 1,
            next_handle: 0x0000_1000,
            tls_slots: HashMap::new(),
            next_tls_index: 1,
            valloc_bump: 0x4000_0000,
            phys_bump: 0x9010_0000,
            xam_app_registered: false,
            online_schema_ptr: 0,
            rng_state: 0x9E37_79B9_7F4A_7C15,
            xam_app_id: 0,
            signin_state: 1,
            perf_counter: 10_000_000,
            ring_buffer_ptr: 0x1000_0000,
            ring_buffer_size: 64 * 1024,
            swap_count: 0,
            last_swap_frontbuffer: 0x7000_0000,
            rptr_writeback_ptr: 0,
            graphics_interrupt_callback: 0,
            graphics_interrupt_context: 0,
            frontbuffer_ptr: 0,
            backbuffer_ptr: 0,
            gpu_identifier_ptr: 0,
            gamepad_state: crate::kernel::input::XInputGamepad::default(),
            gamepad_packet_number: 1,
            pending_threads: Vec::new(),
            events: HashMap::new(),
            event_by_ptr: HashMap::new(),
            vfs_files: HashMap::new(),
            modules: super::modules::ModuleTable::default(),
            vfs_handles: HashMap::new(),
        }
    }

    /// Registers a file in the virtual filesystem with normalized case-insensitive aliases
    pub fn register_vfs_file(&mut self, path: &str, data: Vec<u8>) {
        let clean_path = path.trim_matches('\0').replace('/', "\\");
        let lower_path = clean_path.to_lowercase();

        // 1. Full lowercase path
        self.vfs_files.insert(lower_path.clone(), data.clone());

        // 2. Just the filename and standard prefixes
        if let Some(filename) = lower_path.rsplit('\\').next() {
            self.vfs_files.insert(filename.to_string(), data.clone());
            self.vfs_files.insert(format!("\\{}", filename), data.clone());
            self.vfs_files.insert(format!("game:\\{}", filename), data.clone());
            self.vfs_files.insert(format!("\\device\\cdrom0\\{}", filename), data.clone());
            self.vfs_files.insert(format!("\\device\\harddisk0\\partition1\\{}", filename), data.clone());
            self.vfs_files.insert(format!("\\device\\flash\\{}", filename), data.clone());
        }

        // 3. Normalized without leading backslash
        if lower_path.starts_with('\\') {
            self.vfs_files.insert(lower_path[1..].to_string(), data.clone());
        }
    }

    /// Finds a file in the virtual filesystem with fallback to on-disk assets
    pub fn find_vfs_file(&mut self, query: &str) -> Option<Vec<u8>> {
        let clean = query.trim_matches('\0').replace('/', "\\");
        let lower = clean.to_lowercase();

        // 1. Direct match
        if let Some(data) = self.vfs_files.get(&lower) {
            return Some(data.clone());
        }

        // 2. Try stripping standard prefixes
        let prefixes = [
            "\\device\\harddisk0\\partition1\\",
            "\\device\\harddisk0\\partition0\\",
            "\\device\\cdrom0\\",
            "\\device\\flash\\",
            "game:\\",
            "game:/",
            "systemroot:\\",
            "\\??\\",
            "\\\\?\\",
            ".\\",
            "\\",
        ];

        let mut stripped = lower.as_str();
        for p in &prefixes {
            if stripped.starts_with(p) {
                stripped = &stripped[p.len()..];
            }
        }

        if let Some(data) = self.vfs_files.get(stripped) {
            return Some(data.clone());
        }

        // 3. Try just the filename
        if let Some(fname) = stripped.rsplit('\\').next() {
            if let Some(data) = self.vfs_files.get(fname) {
                return Some(data.clone());
            }

            // 4. Fallback to host filesystem at ../extracted_xui
            let disk_paths = [
                format!("../extracted_xui/{}", stripped),
                format!("../extracted_xui/{}", fname),
                format!("../extracted_xui/pt-br/{}", fname),
                format!("../extracted_xui/named/{}", fname),
            ];

            for dp in &disk_paths {
                if let Ok(d) = std::fs::read(dp) {
                    println!("  [VFS] Loaded file on-demand from host disk '{}' ({} bytes)", dp, d.len());
                    self.register_vfs_file(fname, d.clone());
                    return Some(d);
                }
            }
        }

        None
    }

    pub fn update_gamepad(&mut self, gamepad: crate::kernel::input::XInputGamepad) {
        if self.gamepad_state.w_buttons != gamepad.w_buttons
            || self.gamepad_state.b_left_trigger != gamepad.b_left_trigger
            || self.gamepad_state.b_right_trigger != gamepad.b_right_trigger
            || self.gamepad_state.s_thumb_lx != gamepad.s_thumb_lx
            || self.gamepad_state.s_thumb_ly != gamepad.s_thumb_ly
            || self.gamepad_state.s_thumb_rx != gamepad.s_thumb_rx
            || self.gamepad_state.s_thumb_ry != gamepad.s_thumb_ry
        {
            self.gamepad_packet_number = self.gamepad_packet_number.wrapping_add(1);
            self.gamepad_state = gamepad;
        }
    }

    pub fn resolve_xboxkrnl(&self, ordinal: u16) -> Option<&'static str> {
        get_xboxkrnl_name(ordinal)
    }

    pub fn resolve_xam(&self, ordinal: u16) -> Option<&'static str> {
        get_xam_name(ordinal)
    }

    /// Returns the currently active frontbuffer address for presentation,
    /// alternating between frontbuffer and backbuffer across VdSwap cycles.
    pub fn active_frontbuffer(&self) -> u64 {
        if self.frontbuffer_ptr != 0 && self.backbuffer_ptr != 0 {
            if self.swap_count % 2 == 0 {
                self.frontbuffer_ptr
            } else {
                self.backbuffer_ptr
            }
        } else if self.frontbuffer_ptr != 0 {
            self.frontbuffer_ptr
        } else if self.last_swap_frontbuffer != 0 && self.last_swap_frontbuffer != 0x7000_0000 {
            self.last_swap_frontbuffer
        } else {
            0x7000_0000
        }
    }

    /// Dispatches a kernel/XAM syscall by ordinal and performs memory reads/writes
    pub fn dispatch(
        &mut self,
        library: &str,
        ordinal: u16,
        args: &[u64],
        bus: &mut MemoryBus,
    ) -> u64 {
        let is_krnl = library.eq_ignore_ascii_case("xboxkrnl.exe");
        let func_name = if is_krnl {
            self.resolve_xboxkrnl(ordinal).unwrap_or("UnknownExport")
        } else {
            self.resolve_xam(ordinal).unwrap_or("UnknownExport")
        };

        if crate::diag::config().log_syscalls {
            println!(
                "  [HLE SYSCALL] {}!{} (#0x{:04X} / {}) with args: {:016X?}",
                library, func_name, ordinal, ordinal, args
            );
        }

        if is_krnl {
            self.dispatch_xboxkrnl(ordinal, func_name, args, bus)
        } else {
            self.dispatch_xam(ordinal, func_name, args, bus)
        }
    }
}
