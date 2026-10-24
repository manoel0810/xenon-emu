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

/// Helper function to extract path string from guest memory (OBJECT_ATTRIBUTES or ANSI_STRING or C-string)
fn read_vfs_path(p_obj_attr: u64, bus: &MemoryBus) -> Option<String> {
    if p_obj_attr == 0 {
        return None;
    }
    // Check if p_obj_attr has ObjectName pointer at offset 4
    let name_ptr = bus.read_u32(p_obj_attr + 4).unwrap_or(0) as u64;
    if name_ptr >= 0x1000 {
        // Try reading as ANSI_STRING: length at +0 (u16), max_length at +2 (u16), buffer at +4 (u32)
        let str_len = bus.read_u16(name_ptr).unwrap_or(0) as usize;
        let buf_ptr = bus.read_u32(name_ptr + 4).unwrap_or(0) as u64;
        if str_len > 0 && str_len < 1024 && buf_ptr >= 0x1000 {
            let mut bytes = Vec::new();
            for i in 0..str_len {
                if let Ok(b) = bus.read_u8(buf_ptr + i as u64) {
                    if b == 0 { break; }
                    bytes.push(b);
                }
            }
            if let Ok(s) = String::from_utf8(bytes.clone()) {
                if !s.is_empty() {
                    return Some(s);
                }
            } else {
                let s: String = bytes.iter().map(|&b| b as char).collect();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }

        // Also try reading name_ptr directly as null-terminated ASCII string
        let mut bytes = Vec::new();
        for i in 0..256 {
            if let Ok(b) = bus.read_u8(name_ptr + i as u64) {
                if b == 0 { break; }
                bytes.push(b);
            } else {
                break;
            }
        }
        if bytes.len() >= 3 && bytes.iter().all(|&b| (0x20..=0x7E).contains(&b) || b == b'\\' || b == b'/') {
            if let Ok(s) = String::from_utf8(bytes) {
                return Some(s);
            }
        }
    }

    // Fallback: Check if p_obj_attr itself is an ANSI_STRING
    let str_len = bus.read_u16(p_obj_attr).unwrap_or(0) as usize;
    let buf_ptr = bus.read_u32(p_obj_attr + 4).unwrap_or(0) as u64;
    if str_len > 0 && str_len < 1024 && buf_ptr >= 0x1000 {
        let mut bytes = Vec::new();
        for i in 0..str_len {
            if let Ok(b) = bus.read_u8(buf_ptr + i as u64) {
                if b == 0 { break; }
                bytes.push(b);
            }
        }
        if let Ok(s) = String::from_utf8(bytes) {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // Fallback: Check if p_obj_attr is directly a null-terminated ASCII string
    let mut bytes = Vec::new();
    for i in 0..256 {
        if let Ok(b) = bus.read_u8(p_obj_attr + i as u64) {
            if b == 0 { break; }
            bytes.push(b);
        } else {
            break;
        }
    }
    if bytes.len() >= 3 && bytes.iter().all(|&b| (0x20..=0x7E).contains(&b) || b == b'\\' || b == b'/') {
        if let Ok(s) = String::from_utf8(bytes) {
            return Some(s);
        }
    }

    None
}

/// Helper function to read null-terminated C string from guest memory
fn read_c_string(ptr: u64, bus: &MemoryBus) -> Option<String> {
    if ptr == 0 { return None; }
    let mut bytes = Vec::new();
    for i in 0..512 {
        if let Ok(b) = bus.read_u8(ptr + i as u64) {
            if b == 0 { break; }
            bytes.push(b);
        } else {
            break;
        }
    }
    if bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).to_string())
    }
}

impl KernelDispatcher {
    fn dispatch_xboxkrnl(
        &mut self,
        ordinal: u16,
        _func_name: &str,
        args: &[u64],
        bus: &mut MemoryBus,
    ) -> u64 {
        match ordinal {
            // 0x0001: DbgBreakPoint
            0x0001 => 0,

            // 0x0003: DbgPrint
            0x0003 => {
                if let Some(&ptr) = args.first() {
                    if ptr != 0 {
                        let mut msg = String::new();
                        for i in 0..256 {
                            if let Ok(b) = bus.read_u8(ptr + i) {
                                if b == 0 {
                                    break;
                                }
                                msg.push(b as char);
                            }
                        }
                        if !msg.is_empty() {
                            println!("    >>> [GUEST DbgPrint] {}", msg.trim_end());
                        }
                    }
                }
                0
            }

            // 0x000D: ExCreateThread
            // (pHandle, stackSize, pThreadId, apiStartup, startRoutine, startContext, creationFlags)
            0x000D => {
                let h_thread = self.next_handle;
                self.next_handle += 4;
                let tid = self.next_thread_id;
                self.next_thread_id += 1;

                if let Some(&p_handle) = args.first() {
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h_thread);
                    }
                }
                if let Some(&p_tid) = args.get(2) {
                    if p_tid != 0 {
                        let _ = bus.write_u32(p_tid, tid);
                    }
                }

                let start_routine = args.get(4).copied().unwrap_or(0);
                let start_context = args.get(5).copied().unwrap_or(0);
                let stack_size = args.get(1).copied().unwrap_or(0x10000).max(0x10000) as u32;
                let creation_flags = args.get(6).copied().unwrap_or(0) as u32;

                println!("    -> ExCreateThread: Created thread TID=0x{:X}, Handle=0x{:X}, Entry=0x{:08X}, Param=0x{:08X}, Stack=0x{:X}",
                    tid, h_thread, start_routine, start_context, stack_size
                );

                if start_routine != 0 {
                    self.pending_threads.push(PendingThread {
                        tid,
                        handle: h_thread,
                        start_routine,
                        start_context,
                        stack_size,
                        creation_flags,
                    });
                }

                0 // STATUS_SUCCESS
            }

            // 0x0007: ExAcquireReadWriteLockExclusive | 0x0008: ExAcquireReadWriteLockShared
            0x0007 | 0x0008 => 0,

            // 0x000F: ExFreePool
            0x000F => 0,

            // 0x0010: ExGetXConfigSetting
            // (category, setting, pBuffer, size, pRequiredSize)
            0x0010 => {
                if let Some(&p_req) = args.get(4) {
                    if p_req != 0 {
                        let _ = bus.write_u16(p_req, 4);
                    }
                }
                if let (Some(&p_buf), Some(&sz)) = (args.get(2), args.get(3)) {
                    if p_buf != 0 && sz >= 4 {
                        let _ = bus.write_u32(p_buf, 1);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x0011: ExInitializeReadWriteLock
            0x0011 => {
                if let Some(&p_lock) = args.first() {
                    if p_lock != 0 {
                        for off in (0..16).step_by(4) {
                            let _ = bus.write_u32(p_lock + off, 0);
                        }
                    }
                }
                0
            }

            // 0x0016: ExReleaseReadWriteLock
            0x0016 => 0,

            // 0x0017: ExRegisterTitleTerminateNotification | 0x0018: ExUnregisterTitleTerminateNotification
            0x0017 | 0x0018 => 0,

            // 0x0019: ExTerminateThread
            0x0019 => 0,

            // 0x0021: FscSetCacheElementCount | 0x0022: FscSetMaxCacheElements
            0x0021 | 0x0022 => 0,

            // 0x0024: HalOpenCloseODDTray
            0x0024 => 0,

            // 0x0029: HalSendSMCMessage
            0x0029 => 0,

            // 0x002C: HalReturnToFirmware
            0x002C => 0,

            // 0x004D: KeAcquireSpinLockAtRaisedIrql
            0x004D => 0,

            // 0x0052: KeBugCheck | 0x0053: KeBugCheckEx
            0x0052 | 0x0053 => {
                println!("    [!] GUEST KeBugCheck called with code 0x{:X}", args.first().unwrap_or(&0));
                0
            }

            // 0x0057: KeDelayExecutionThread (waitMode, alertable, pInterval)
            0x0057 => 0,

            // 0x005F: KeEnterCriticalRegion
            0x005F => 0,

            // 0x0066: KeGetCurrentProcessType
            // Returns 2 for System process (Dashboard), 1 for Title
            0x0066 => 2,

            // 0x006B: KeLockL2 | 0x006C: KeUnlockL2
            0x006B | 0x006C => 0,

            // 0x0070: KeInitializeEvent (pEvent, eventType, initialState)
            0x0070 => {
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        let event_type = args.get(1).copied().unwrap_or(0) as u32;
                        let initial = args.get(2).copied().unwrap_or(0) as u32;

                        // KEVENT memory layout:
                        // 0x00: Header.Type (0 = Notification, 1 = Synchronization)
                        // 0x01: Header.Absolute
                        // 0x02: Header.Size (4 dwords = 16 bytes)
                        // 0x03: Header.Inserted
                        // 0x04: Header.SignalState
                        // 0x08: WaitListHead.Flink
                        // 0x0C: WaitListHead.Blink
                        let _ = bus.write_u8(p_event, event_type as u8);
                        let _ = bus.write_u8(p_event + 1, 0);
                        let _ = bus.write_u8(p_event + 2, 4);
                        let _ = bus.write_u8(p_event + 3, 0);
                        let _ = bus.write_u32(p_event + 4, initial);
                        let _ = bus.write_u32(p_event + 8, (p_event + 8) as u32);
                        let _ = bus.write_u32(p_event + 12, (p_event + 8) as u32);

                        let h = self.next_handle;
                        self.next_handle += 4;
                        self.events.insert(h, KernelEvent {
                            handle: h,
                            event_type,
                            signaled: initial != 0,
                            guest_ptr: p_event,
                        });
                        self.event_by_ptr.insert(p_event, h);
                    }
                }
                0
            }

            // 0x006F: KeInitializeDpc (pDpc, pRoutine, pContext)
            0x006F => {
                if let (Some(&p_dpc), Some(&p_routine), Some(&p_context)) = (args.first(), args.get(1), args.get(2)) {
                    if p_dpc != 0 {
                        let _ = bus.write_u32(p_dpc + 8, p_routine as u32);
                        let _ = bus.write_u32(p_dpc + 12, p_context as u32);
                    }
                }
                0
            }

            // 0x0074: KeInitializeSemaphore (pSemaphore, count, limit)
            0x0074 => {
                if let Some(&p_sem) = args.first() {
                    if p_sem != 0 {
                        for off in (0..20).step_by(4) {
                            let _ = bus.write_u32(p_sem + off, 0);
                        }
                        let count = args.get(1).copied().unwrap_or(1) as u32;
                        let _ = bus.write_u32(p_sem + 4, count);
                    }
                }
                0
            }

            // 0x007D: KeLeaveCriticalRegion
            0x007D => 0,

            // 0x007F: KePulseEvent (pEvent, increment, wait)
            0x007F => {
                let mut prev = 0u64;
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        if let Ok(st) = bus.read_u32(p_event + 4) {
                            prev = st as u64;
                        }
                        let _ = bus.write_u32(p_event + 4, 0);
                        if let Some(&h) = self.event_by_ptr.get(&p_event) {
                            if let Some(ev) = self.events.get_mut(&h) {
                                ev.signaled = false;
                            }
                        }
                    }
                }
                prev
            }

            // 0x0080: KeQueryActiveProcessors
            0x0080 => 0x3F, // 6 hardware threads (3 cores x 2 SMT)

            // 0x0081: KeQueryBasePriorityThread
            0x0081 => 8, // Normal thread priority

            // 0x0085: KeRaiseIrqlToDpcLevel
            0x0085 => 2, // Previous IRQL (DISPATCH_LEVEL)

            // 0x0088: KeReleaseSemaphore
            0x0088 => 1,

            // 0x0083: KeQueryPerformanceFrequency (pFrequency)
            0x0083 => {
                if let Some(&p_freq) = args.first() {
                    if p_freq != 0 {
                        let _ = bus.write_u64(p_freq, 50_000_000);
                    }
                }
                1 // TRUE
            }

            // 0x0084: KeQuerySystemTime (pCurrentTime)
            0x0084 => {
                if let Some(&p_time) = args.first() {
                    if p_time != 0 {
                        let ft = 133_000_000_000_000_000u64 + (self.next_handle as u64 * 10_000_000);
                        let _ = bus.write_u64(p_time, ft);
                    }
                }
                0
            }

            // 0x0089: KeReleaseSpinLockFromRaisedIrql
            0x0089 => 0,

            // 0x008D: KeRemoveQueue | 0x0090: KeRundownQueue | 0x0095: KeInsertQueue | 0x0096: KeInsertHeadQueue | 0x0098: KeSetCurrentStackPointers
            0x008D | 0x0090 | 0x0095 | 0x0096 | 0x0098 => 0,

            // 0x008F: KeResetEvent
            0x008F => {
                let mut prev = 0u64;
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        if let Ok(st) = bus.read_u32(p_event + 4) {
                            prev = st as u64;
                        }
                        let _ = bus.write_u32(p_event + 4, 0);
                        if let Some(&h) = self.event_by_ptr.get(&p_event) {
                            if let Some(ev) = self.events.get_mut(&h) {
                                ev.signaled = false;
                            }
                        }
                    }
                }
                prev
            }

            // 0x0092: KeResumeThread
            0x0092 => 1, // Previous suspend count

            // 0x0097: KeSetAffinityThread
            0x0097 => 1,

            // 0x0099: KeSetBasePriorityThread
            0x0099 => 0,

            // 0x009D: KeSetEvent
            0x009D => {
                let mut prev = 0u64;
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        if let Ok(st) = bus.read_u32(p_event + 4) {
                            prev = st as u64;
                        }
                        let _ = bus.write_u32(p_event + 4, 1);
                        if let Some(&h) = self.event_by_ptr.get(&p_event) {
                            if let Some(ev) = self.events.get_mut(&h) {
                                ev.signaled = true;
                            }
                        }
                    }
                }
                prev
            }

            // 0x00AF: KeWaitForMultipleObjects
            0x00AF => {
                let count = args.first().copied().unwrap_or(1) as usize;
                let p_objects = args.get(1).copied().unwrap_or(0);
                if p_objects != 0 {
                    for i in 0..count.min(8) {
                        if let Ok(p_obj) = bus.read_u32(p_objects + (i * 4) as u64) {
                            if p_obj != 0 {
                                if let Ok(typ) = bus.read_u8(p_obj as u64) {
                                    if typ == 1 {
                                        let _ = bus.write_u32(p_obj as u64 + 4, 0);
                                    }
                                }
                            }
                        }
                    }
                }
                0 // STATUS_WAIT_0 / STATUS_SUCCESS
            }

            // 0x00B0: KeWaitForSingleObject
            0x00B0 => {
                if let Some(&p_obj) = args.first() {
                    if p_obj != 0 {
                        let is_sync = bus.read_u8(p_obj).unwrap_or(0) == 1;
                        if is_sync {
                            let _ = bus.write_u32(p_obj + 4, 0);
                        }
                    }
                }
                0 // STATUS_SUCCESS / WAIT_OBJECT_0
            }

            // 0x00B1: KfAcquireSpinLock | 0x00B2: KfRaiseIrql
            0x00B1 | 0x00B2 => 0,

            // 0x00B3: KfLowerIrql
            0x00B3 => 0,

            // 0x00B4: KfReleaseSpinLock
            0x00B4 => 0,

            // 0x00BA: MmAllocatePhysicalMemoryEx
            // (type, size, protect, minAddr, maxAddr, alignment)
            0x00BA => {
                let size = args.get(1).copied().unwrap_or(0x1000).max(0x1000);
                let aligned = (size + 0xFFF) & !0xFFF;
                let addr = self.phys_bump;
                self.phys_bump += aligned;
                if self.phys_bump > 0x9200_0000 {
                    println!("    -> MmAllocatePhysicalMemoryEx: physical bump exceeded 0x92000000 (would overlap the dash image)");
                }
                if aligned == 0x398000 {
                    if self.frontbuffer_ptr == 0 {
                        self.frontbuffer_ptr = addr;
                        println!("    -> MmAllocatePhysicalMemoryEx: Set Frontbuffer @ 0x{:08X}", addr);
                    } else if self.backbuffer_ptr == 0 {
                        self.backbuffer_ptr = addr;
                        println!("    -> MmAllocatePhysicalMemoryEx: Set Backbuffer @ 0x{:08X}", addr);
                    }
                }
                println!("    -> MmAllocatePhysicalMemoryEx: Allocated 0x{:X} bytes at 0x{:08X}", aligned, addr);
                addr
            }

            // 0x00BD: MmFreePhysicalMemory
            0x00BD => 0,

            // 0x00BE: MmGetPhysicalAddress
            0x00BE => args.first().copied().unwrap_or(0) & 0x1FFF_FFFF,

            // 0x00C4: MmQueryAddressProtect
            0x00C4 => 0x04, // PAGE_READWRITE

            // 0x00C5: MmQueryAllocationSize | 0x00C7: MmQueryAllocationSize
            0x00C5 | 0x00C7 => 0x1000,

            // 0x00C8: MmQueryStatistics
            0x00C8 => 0,

            // 0x00CC: NtAllocateVirtualMemory
            // (IN OUT PVOID *BaseAddress, IN OUT PSIZE_T RegionSize, ULONG AllocationType, ULONG Protect, ULONG ZeroBits)
            0x00CC => {
                let p_base = args.get(0).copied().unwrap_or(0);
                let p_size = args.get(1).copied().unwrap_or(0);
                let alloc_type = args.get(2).copied().unwrap_or(0);
                let protect = args.get(3).copied().unwrap_or(0);

                let cur_base = if p_base != 0 {
                    bus.read_u32(p_base).unwrap_or(0) as u64
                } else {
                    0
                };

                let req_size = if p_size != 0 {
                    bus.read_u32(p_size).unwrap_or(0x1000) as u64
                } else {
                    0x1000
                };

                let aligned_size = (req_size.max(0x1000) + 0xFFF) & !0xFFF;

                let target_addr = if cur_base != 0 {
                    cur_base
                } else {
                    let addr = self.valloc_bump;
                    self.valloc_bump += aligned_size;
                    addr
                };

                if p_base != 0 {
                    let _ = bus.write_u32(p_base, target_addr as u32);
                }
                if p_size != 0 {
                    let _ = bus.write_u32(p_size, aligned_size as u32);
                }

                println!(
                    "    -> NtAllocateVirtualMemory: Base=0x{:08X}, Size=0x{:X} (Type=0x{:X}, Prot=0x{:X})",
                    target_addr, aligned_size, alloc_type, protect
                );
                0 // STATUS_SUCCESS
            }

            // 0x00CE: NtClearEvent
            0x00CE => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        ev.signaled = false;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 0);
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00CF: NtClose (Handle)
            0x00CF => {
                let h = args.first().copied().unwrap_or(0) as u32;
                self.vfs_handles.remove(&h);
                self.events.remove(&h);
                0 // STATUS_SUCCESS
            }

            // 0x00D1: NtCreateEvent (pHandle, desiredAccess, pObjAttr, eventType, initialState)
            0x00D1 => {
                let h = self.next_handle;
                self.next_handle += 4;
                let event_type = args.get(3).copied().unwrap_or(0) as u32;
                let initial = args.get(4).copied().unwrap_or(0) as u32;

                if let Some(&p_handle) = args.first() {
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h);
                    }
                }
                self.events.insert(h, KernelEvent {
                    handle: h,
                    event_type,
                    signaled: initial != 0,
                    guest_ptr: 0,
                });
                0 // STATUS_SUCCESS
            }

            // 0x0038: IoCreateFile | 0x00D2: NtCreateFile | 0x00DF: NtOpenFile
            0x0038 | 0x00D2 | 0x00DF => {
                let p_handle = args.first().copied().unwrap_or(0);
                let p_obj_attr = args.get(2).copied().unwrap_or(0);
                let p_io_status = args.get(3).copied().unwrap_or(0);

                let path_opt = read_vfs_path(p_obj_attr, bus);
                if let Some(path) = path_opt {
                    println!("    [VFS] NtCreateFile / NtOpenFile: '{}'", path);
                    if let Some(data) = self.find_vfs_file(&path) {
                        let h = self.next_handle;
                        self.next_handle += 4;
                        let is_dir = path.ends_with('\\') || path.ends_with('/');
                        self.vfs_handles.insert(h, VfsFileHandle {
                            handle: h,
                            name: path.clone(),
                            data,
                            position: 0,
                            is_directory: is_dir,
                        });
                        if p_handle != 0 {
                            let _ = bus.write_u32(p_handle, h);
                        }
                        if p_io_status != 0 {
                            let _ = bus.write_u32(p_io_status, 0); // STATUS_SUCCESS
                            let _ = bus.write_u32(p_io_status + 4, 1); // FILE_OPENED
                        }
                        println!("      -> VFS Handle 0x{:X} granted for '{}'", h, path);
                        0 // STATUS_SUCCESS
                    } else {
                        println!("      -> VFS File NOT found: '{}'", path);
                        if p_io_status != 0 {
                            let _ = bus.write_u32(p_io_status, 0xC000_0034); // STATUS_OBJECT_NAME_NOT_FOUND
                            let _ = bus.write_u32(p_io_status + 4, 0);
                        }
                        0xC000_0034 // STATUS_OBJECT_NAME_NOT_FOUND
                    }
                } else {
                    let h = self.next_handle;
                    self.next_handle += 4;
                    self.vfs_handles.insert(h, VfsFileHandle {
                        handle: h,
                        name: "<unnamed>".to_string(),
                        data: Vec::new(),
                        position: 0,
                        is_directory: false,
                    });
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h);
                    }
                    if p_io_status != 0 {
                        let _ = bus.write_u32(p_io_status, 0);
                        let _ = bus.write_u32(p_io_status + 4, 1);
                    }
                    0 // STATUS_SUCCESS
                }
            }

            // 0x00D3: NtCreateIoCompletion
            0x00D3 => {
                let handle = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.first() {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, handle);
                    }
                }
                0
            }

            // 0x00D5: NtCreateSemaphore
            0x00D5 => {
                let handle = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.first() {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, handle);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00D8: NtDeleteFile
            0x00D8 => 0,

            // 0x00D9: NtDeviceIoControlFile
            0x00D9 => {
                if let Some(&p_ios) = args.get(4) {
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 0);
                    }
                }
                0
            }

            // 0x00DB: NtFlushBuffersFile
            0x00DB => 0,

            // 0x00DC: NtFreeVirtualMemory
            0x00DC => 0,

            // 0x00E2: NtPulseEvent
            0x00E2 => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        ev.signaled = false;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 0);
                        }
                    }
                }
                0
            }

            // 0x00E7: NtQueryFullAttributesFile (pObjAttr, pNetworkOpenInfo)
            0x00E7 => {
                let p_obj_attr = args.first().copied().unwrap_or(0);
                let p_info = args.get(1).copied().unwrap_or(0);
                if let Some(path) = read_vfs_path(p_obj_attr, bus) {
                    if let Some(data) = self.find_vfs_file(&path) {
                        if p_info != 0 {
                            let flen = data.len() as u64;
                            let _ = bus.write_u64(p_info + 0, 0); // CreationTime
                            let _ = bus.write_u64(p_info + 8, 0); // LastAccessTime
                            let _ = bus.write_u64(p_info + 16, 0); // LastWriteTime
                            let _ = bus.write_u64(p_info + 24, 0); // ChangeTime
                            let _ = bus.write_u64(p_info + 32, flen); // AllocationSize
                            let _ = bus.write_u64(p_info + 40, flen); // EndOfFile
                            let _ = bus.write_u32(p_info + 48, 0x80); // FILE_ATTRIBUTE_NORMAL
                        }
                        0 // STATUS_SUCCESS
                    } else {
                        0xC000_0034 // STATUS_OBJECT_NAME_NOT_FOUND
                    }
                } else {
                    0xC000_000D // STATUS_INVALID_PARAMETER
                }
            }

            // 0x00E8: NtQueryInformationFile (FileHandle, IoStatusBlock, FileInformation, Length, FileInformationClass)
            0x00E8 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_ios = args.get(1).copied().unwrap_or(0);
                let p_info = args.get(2).copied().unwrap_or(0);
                let info_class = args.get(4).copied().unwrap_or(0) as u32;

                if let Some(file) = self.vfs_handles.get(&h) {
                    let flen = file.data.len() as u64;
                    let pos = file.position as u64;
                    match info_class {
                        5 => { // FileStandardInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, flen); // AllocationSize
                                let _ = bus.write_u64(p_info + 8, flen); // EndOfFile (size)
                                let _ = bus.write_u32(p_info + 16, 1);   // NumberOfLinks
                                let _ = bus.write_u8(p_info + 20, 0);    // DeletePending
                                let _ = bus.write_u8(p_info + 21, if file.is_directory { 1 } else { 0 }); // Directory
                            }
                        }
                        14 => { // FilePositionInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, pos); // CurrentByteOffset
                            }
                        }
                        4 => { // FileBasicInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, 0);
                                let _ = bus.write_u64(p_info + 8, 0);
                                let _ = bus.write_u64(p_info + 16, 0);
                                let _ = bus.write_u64(p_info + 24, 0);
                                let _ = bus.write_u32(p_info + 32, 0x80);
                            }
                        }
                        21 => { // FileNetworkOpenInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, 0);
                                let _ = bus.write_u64(p_info + 8, 0);
                                let _ = bus.write_u64(p_info + 16, 0);
                                let _ = bus.write_u64(p_info + 24, 0);
                                let _ = bus.write_u64(p_info + 32, flen);
                                let _ = bus.write_u64(p_info + 40, flen);
                                let _ = bus.write_u32(p_info + 48, 0x80);
                            }
                        }
                        _ => {
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, flen);
                                let _ = bus.write_u64(p_info + 8, flen);
                            }
                        }
                    }
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 24);
                    }
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00EE: NtQueryVirtualMemory
            0x00EE => 0,

            // 0x00EF: NtQueryVolumeInformationFile
            0x00EF => {
                if let Some(&p_ios) = args.get(1) {
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 0);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00F0: NtReadFile (FileHandle, Event, ApcRoutine, ApcContext, IoStatusBlock, Buffer, Length, ByteOffset, Key)
            0x00F0 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let event_h = args.get(1).copied().unwrap_or(0) as u32;
                let p_ios = args.get(4).copied().unwrap_or(0);
                let p_buf = args.get(5).copied().unwrap_or(0);
                let len = args.get(6).copied().unwrap_or(0) as usize;
                let p_offset = args.get(7).copied().unwrap_or(0);

                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    if p_offset != 0 {
                        let off = bus.read_u64(p_offset).unwrap_or(0) as usize;
                        file.position = off;
                    }
                    if file.position >= file.data.len() && len > 0 {
                        if p_ios != 0 {
                            let _ = bus.write_u32(p_ios, 0xC000_0011); // STATUS_END_OF_FILE
                            let _ = bus.write_u32(p_ios + 4, 0);
                        }
                        return 0xC000_0011;
                    }
                    let avail = file.data.len().saturating_sub(file.position);
                    let to_read = std::cmp::min(len, avail);
                    if to_read > 0 && p_buf != 0 {
                        let slice = &file.data[file.position..file.position + to_read];
                        let _ = bus.load_bytes(p_buf, slice);
                        file.position += to_read;
                    }
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, to_read as u32);
                    }
                    if event_h != 0 {
                        if let Some(ev) = self.events.get_mut(&event_h) {
                            ev.signaled = true;
                            if ev.guest_ptr != 0 {
                                let _ = bus.write_u32(ev.guest_ptr + 4, 1);
                            }
                        }
                    }
                    println!("    [VFS] NtReadFile: read {} bytes from '{}' (pos: {}/{})", to_read, file.name, file.position, file.data.len());
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00F3: NtReleaseSemaphore (hSemaphore, releaseCount, pPreviousCount)
            0x00F3 => {
                if let Some(&p_prev) = args.get(2) {
                    if p_prev != 0 {
                        let _ = bus.write_u32(p_prev, 1);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00F4: NtRemoveIoCompletion
            0x00F4 => 0x0000_0102, // STATUS_TIMEOUT

            // 0x00F6: NtSetEvent (hEvent, pPrevState)
            0x00F6 => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    let prev = if let Some(ev) = self.events.get_mut(&h32) {
                        let p = ev.signaled;
                        ev.signaled = true;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 1);
                        }
                        if p { 1 } else { 0 }
                    } else {
                        0
                    };
                    if let Some(&p_prev) = args.get(1) {
                        if p_prev != 0 {
                            let _ = bus.write_u32(p_prev, prev);
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00F7: NtSetInformationFile (FileHandle, IoStatusBlock, FileInformation, Length, FileInformationClass)
            0x00F7 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_ios = args.get(1).copied().unwrap_or(0);
                let p_info = args.get(2).copied().unwrap_or(0);
                let info_class = args.get(4).copied().unwrap_or(0) as u32;

                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    if info_class == 14 && p_info != 0 {
                        let new_pos = bus.read_u64(p_info).unwrap_or(0) as usize;
                        file.position = new_pos;
                    }
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 0);
                    }
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00F8: NtSetIoCompletion
            0x00F8 => 0,

            // 0x00FB: NtSignalAndWaitForSingleObjectEx
            0x00FB => {
                if let Some(&h1) = args.first() {
                    let h32 = h1 as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        ev.signaled = true;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 1);
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00FC: NtSuspendThread
            0x00FC => 0,

            // 0x00FD: NtWaitForSingleObjectEx
            0x00FD => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        if ev.event_type == 1 {
                            ev.signaled = false;
                            if ev.guest_ptr != 0 {
                                let _ = bus.write_u32(ev.guest_ptr + 4, 0);
                            }
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00FE: NtWaitForMultipleObjectsEx
            0x00FE => 0, // STATUS_WAIT_0

            // 0x00FF: NtWriteFile
            0x00FF => 0,

            // 0x0101: NtYieldExecution
            0x0101 => 0,

            // 0x0103: ObCreateSymbolicLink | 0x0104: ObDeleteSymbolicLink | 0x0105: ObDereferenceObject
            0x0103 | 0x0104 | 0x0105 => 0,

            // 0x010B: ObLookupThreadByThreadId
            0x010B => {
                if let Some(&p_obj) = args.get(1) {
                    if p_obj != 0 {
                        let _ = bus.write_u32(p_obj, 0x8001_0000);
                    }
                }
                0
            }

            // 0x010E: ObOpenObjectByPointer
            0x010E => {
                let handle = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.get(3) {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, handle);
                    }
                }
                0
            }

            // 0x0110: ObReferenceObjectByHandle
            0x0110 => {
                if let Some(&p_obj) = args.get(3) {
                    if p_obj != 0 {
                        let _ = bus.write_u32(p_obj, 0x8001_0000);
                    }
                }
                0
            }

            // 0x0119: RtlCaptureContext
            0x0119 => 0,

            // 0x011B: RtlCompareMemoryUlong
            0x011B => args.get(1).copied().unwrap_or(0),

            // 0x0125: RtlEnterCriticalSection
            0x0125 => 0,

            // 0x0126: RtlFillMemoryUlong (Destination, Length in BYTES, Pattern)
            // (call site in the dash: r3=dst, r4=800, r5=0x80000000 -> Length=800, Pattern=0x80000000.
            //  The previous handler read (dst, pattern, count) and looped 2^64 times on the sign-extended pattern.)
            0x0126 => {
                let dst = args.first().copied().unwrap_or(0);
                let len = (args.get(1).copied().unwrap_or(0) & 0xFFFF_FFFF) as u64;
                let pat = args.get(2).copied().unwrap_or(0) as u32;
                // Defensive cap: a single fill can never legitimately exceed the whole 512 MB RAM
                let words = (len / 4).min(128 * 1024 * 1024);
                for i in 0..words {
                    let _ = bus.write_u32(dst.wrapping_add(i * 4), pat);
                }
                0
            }
            // 0x012B: RtlImageXexHeaderField
            0x012B => 0,

            // 0x012C: RtlInitAnsiString (DestinationString, SourceString)
            0x012C => {
                let dst = args.first().copied().unwrap_or(0);
                let src = args.get(1).copied().unwrap_or(0);
                if dst != 0 {
                    if src == 0 {
                        let _ = bus.write_u16(dst, 0);
                        let _ = bus.write_u16(dst + 2, 0);
                        let _ = bus.write_u32(dst + 4, 0);
                    } else {
                        let mut len = 0u16;
                        while len < 0xFFFE {
                            if bus.read_u8(src + len as u64).unwrap_or(0) == 0 {
                                break;
                            }
                            len += 1;
                        }
                        let _ = bus.write_u16(dst, len);
                        let _ = bus.write_u16(dst + 2, len.saturating_add(1));
                        let _ = bus.write_u32(dst + 4, src as u32);
                    }
                }
                0
            }

            // 0x012D: RtlInitUnicodeString (DestinationString, SourceString)
            0x012D => {
                let dst = args.first().copied().unwrap_or(0);
                let src = args.get(1).copied().unwrap_or(0);
                if dst != 0 {
                    if src == 0 {
                        let _ = bus.write_u16(dst, 0);
                        let _ = bus.write_u16(dst + 2, 0);
                        let _ = bus.write_u32(dst + 4, 0);
                    } else {
                        let mut len_bytes = 0u16;
                        while len_bytes < 0xFFFC {
                            let ch = bus.read_u16(src + len_bytes as u64).unwrap_or(0);
                            if ch == 0 {
                                break;
                            }
                            len_bytes += 2;
                        }
                        let _ = bus.write_u16(dst, len_bytes);
                        let _ = bus.write_u16(dst + 2, len_bytes.saturating_add(2));
                        let _ = bus.write_u32(dst + 4, src as u32);
                    }
                }
                0
            }

            // 0x012E: RtlInitializeCriticalSection | 0x012F: RtlInitializeCriticalSectionAndSpinCount
            0x012E | 0x012F => {
                if let Some(&ptr) = args.first() {
                    if ptr != 0 {
                        for off in (0..28).step_by(4) {
                            let _ = bus.write_u32(ptr + off, 0);
                        }
                    }
                }
                0
            }

            // 0x0130: RtlLeaveCriticalSection
            0x0130 => 0,

            // 0x0133: RtlMultiByteToUnicodeN
            0x0133 => 0,

            // 0x0135: RtlNtStatusToDosError
            0x0135 => 0, // ERROR_SUCCESS

            // 0x0136: RtlRaiseException
            0x0136 => 0,

            // 0x013A: _snprintf | 0x013B: sprintf
            0x013A | 0x013B => 0,

            // 0x0140: RtlFreeAnsiString | 0x0141: RtlFreeUnicodeString
            0x0140 | 0x0141 => {
                let str_ptr = args.first().copied().unwrap_or(0);
                if str_ptr != 0 {
                    let _ = bus.write_u16(str_ptr, 0);
                    let _ = bus.write_u16(str_ptr + 2, 0);
                    let _ = bus.write_u32(str_ptr + 4, 0);
                }
                0
            }

            // 0x0142: RtlUnicodeStringToAnsiString (DestinationString, SourceString, AllocateDestinationString)
            0x0142 => {
                let dst = args.first().copied().unwrap_or(0);
                let src = args.get(1).copied().unwrap_or(0);
                let alloc_dest = args.get(2).copied().unwrap_or(0) != 0;

                if dst == 0 || src == 0 {
                    0xC000000D // STATUS_INVALID_PARAMETER
                } else {
                    let src_len = bus.read_u16(src).unwrap_or(0);
                    let src_buf = bus.read_u32(src + 4).unwrap_or(0) as u64;
                    let char_count = (src_len / 2) as usize;

                    let mut ansi_bytes = Vec::with_capacity(char_count);
                    for i in 0..char_count {
                        let wch = bus.read_u16(src_buf + (i * 2) as u64).unwrap_or(0);
                        ansi_bytes.push(if wch < 0x80 { wch as u8 } else { b'?' });
                    }

                    let dst_buf = if alloc_dest {
                        let alloc_size = ((char_count + 1 + 31) & !31) as u64;
                        let ptr = self.valloc_bump;
                        self.valloc_bump += alloc_size;
                        let _ = bus.write_u16(dst + 2, (char_count + 1) as u16);
                        let _ = bus.write_u32(dst + 4, ptr as u32);
                        ptr
                    } else {
                        bus.read_u32(dst + 4).unwrap_or(0) as u64
                    };

                    let _ = bus.write_u16(dst, char_count as u16);
                    if dst_buf != 0 {
                        for (i, &b) in ansi_bytes.iter().enumerate() {
                            let _ = bus.write_u8(dst_buf + i as u64, b);
                        }
                        let _ = bus.write_u8(dst_buf + char_count as u64, 0);
                    }

                    0 // STATUS_SUCCESS
                }
            }

            // 0x0143: RtlUnicodeToMultiByteN
            0x0143 => 0,

            // 0x0147: RtlUnwind
            0x0147 => 0,

            // 0x014D: _vsnprintf | 0x014F: _vscwprintf | 0x0151: vswprintf
            0x014D | 0x014F | 0x0151 => 0,

            // 0x0152: KeTlsAlloc
            0x0152 => {
                let slot = self.next_tls_index;
                self.next_tls_index += 1;
                slot as u64
            }

            // 0x0153: KeTlsFree
            0x0153 => 1, // TRUE

            // 0x0154: KeTlsGetValue (slot)
            0x0154 => {
                let slot = args.first().copied().unwrap_or(0) as u32;
                self.tls_slots.get(&slot).copied().unwrap_or(0)
            }

            // 0x0155: KeTlsSetValue (slot, value)
            0x0155 => {
                let slot = args.first().copied().unwrap_or(0) as u32;
                let val = args.get(1).copied().unwrap_or(0);
                if slot == 1 {
                    println!("    [TLS SET] slot=1, val=0x{:X}", val);
                }
                self.tls_slots.insert(slot, val);
                1 // TRUE
            }

            // 0x0158: XboxKrnlVersion
            0x0158 => 0x2011_B400, // 2.0.4532.0 (Genuine Dashboard Kernel)

            // 0x0166: XeCryptBnQwBeSigVerify
            0x0166 => 1, // Signature Valid (TRUE)

            // 0x0174..0x017A: XeCryptDes*
            0x0174 | 0x0175 | 0x0176 | 0x0178 | 0x017A => 0,

            // 0x0182: XeCryptHmacSha | 0x0183: XeCryptHmacShaVerify
            0x0182 | 0x0183 => 1,

            // 0x0187: XeCryptMd5 | 0x018E: XeCryptRotSumSha | 0x0192: XeCryptSha
            0x0187 | 0x018E | 0x0192 => 0,

            // 0x018A: XeCryptRandom (pBuffer, cbBuffer) per the generated export table; the dash calls it 4096x with (buf, 4).
            // (An older comment here called it VeSetHandlers.) Fill the buffer with deterministic xorshift bytes.
            0x018A => {
                let p = args.first().copied().unwrap_or(0);
                let n = (args.get(1).copied().unwrap_or(0) & 0xFFFF_FFFF).min(4096);
                if p != 0 {
                    let mut x = self.rng_state;
                    for i in 0..n {
                        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                        let _ = bus.write_u8(p + i, (x >> 24) as u8);
                    }
                    self.rng_state = x;
                }
                0
            }

            // 0x0194: XexCheckExecutablePrivilege
            0x0194 => 1, // TRUE (Privilege Granted)

            // 0x0195: XexGetModuleHandle
            0x0195 => {
                let mod_name_ptr = args.first().copied().unwrap_or(0);
                if mod_name_ptr == 0 {
                    0x9200_0000 // Self (Dashboard)
                } else {
                    0x9000_0000
                }
            }

            // 0x0196: XexGetModuleSection (hMod, name, pBase, pSize)
            0x0196 => {
                if let Some(&p_base) = args.get(2) {
                    if p_base != 0 {
                        let _ = bus.write_u32(p_base, 0x9210_5000);
                    }
                }
                if let Some(&p_sz) = args.get(3) {
                    if p_sz != 0 {
                        let _ = bus.write_u32(p_sz, 0x0058_A894);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x0197: XexGetProcedureAddress
            0x0197 => 0x9268_DD14,

            // 0x0199: XexLoadImage | 0x01A1: XexUnloadImage | 0x01A2: XexUnloadImageAndExitThread | 0x01A4: XexVerifyImageHeaders
            0x0199 | 0x01A1 | 0x01A2 | 0x01A4 => 0,

            // 0x01A5: __C_specific_handler
            0x01A5 => 0,

            // 0x01B1..0x01B9: Video Display (Vd*) Handlers
            0x01B1 | 0x01B3 | 0x01B4 | 0x01B5 | 0x01B7 | 0x01B8 | 0x01B9 => 0,

            // 0x01B6: VdEnableRingBufferRPtrWriteBack (pWriteBackAddress, frequency)
            0x01B6 => {
                if let Some(&ptr) = args.first() {
                    self.rptr_writeback_ptr = ptr;
                    println!("    -> VdEnableRingBufferRPtrWriteBack: Address=0x{:08X}", ptr);
                }
                0
            }

            // 0x01BA: VdGetCurrentDisplayInformation
            0x01BA => {
                if let Some(&p_mode) = args.first() {
                    if p_mode != 0 {
                        let _ = bus.write_u32(p_mode + 0, 1280); // Width
                        let _ = bus.write_u32(p_mode + 4, 720);  // Height
                        let _ = bus.write_u32(p_mode + 8, 0);    // Interlaced (0=Progressive)
                        let _ = bus.write_u32(p_mode + 12, 1);   // Widescreen (1=16:9)
                        let _ = bus.write_u32(p_mode + 16, 60);  // RefreshRate (60Hz)
                    }
                }
                0
            }

            // 0x01BD: VdGetSystemCommandBuffer
            0x01BD => 0x1000_0000,

            // 0x01BE: VdGlobalDevice
            0x01BE => 0x1001_0000,

            // 0x01C0: VdGpuClockInMHz
            0x01C0 => 500, // 500 MHz ATI Xenos

            // 0x01C1: VdHSIOCalibrationLock
            0x01C1 => 0,

            // 0x01C2: VdInitializeEngines
            0x01C2 => 0,

            // 0x01C3: VdInitializeRingBuffer
            0x01C3 => {
                if let Some(&ptr) = args.first() {
                    if ptr != 0 {
                        self.ring_buffer_ptr = ptr;
                    }
                }
                if let Some(&size) = args.get(1) {
                    if size != 0 {
                        let bytes = if size <= 32 {
                            let comp = (1u64 << size) * 4;
                            comp.max(32 * 1024) as u32
                        } else {
                            size as u32
                        };
                        self.ring_buffer_size = bytes;
                    }
                }
                println!("    -> VdInitializeRingBuffer: Address=0x{:08X}, Size=0x{:X} ({} KB)",
                    self.ring_buffer_ptr, self.ring_buffer_size, self.ring_buffer_size / 1024);
                0
            }

            // 0x01C5: VdInitializeScalerCommandBuffer
            0x01C5 => 0,

            // 0x01C6: VdIsHSIOTrainingSucceeded
            0x01C6 => 1, // TRUE

            // 0x01C7: VdPersistDisplay
            0x01C7 => 0,

            // 0x01C9: VdQueryVideoFlags
            0x01C9 => 0x0000_0001, // Widescreen 16:9

            // 0x01CA: VdQueryVideoMode
            // Fills XVIDEO_MODE at args[0]: (1280x720, 60Hz, 16:9)
            0x01CA => {
                if let Some(&p_mode) = args.first() {
                    if p_mode != 0 {
                        let _ = bus.write_u32(p_mode + 0, 1280); // Width
                        let _ = bus.write_u32(p_mode + 4, 720);  // Height
                        let _ = bus.write_u32(p_mode + 8, 0);    // Interlaced (0=Progressive)
                        let _ = bus.write_u32(p_mode + 12, 1);   // Widescreen (1=16:9)
                        let _ = bus.write_u32(p_mode + 16, 60);  // RefreshRate (60Hz)
                    }
                }
                0
            }

            // 0x01CF..0x01DC: Vd display controls
            0x01CF | 0x01D0 | 0x01D3 | 0x01D4 | 0x01DC => 0,

            // 0x01D5: VdSetGraphicsInterruptCallback (pfnCallback, pContext)
            0x01D5 => {
                if let Some(&cb) = args.first() {
                    self.graphics_interrupt_callback = cb;
                }
                if let Some(&ctx) = args.get(1) {
                    self.graphics_interrupt_context = ctx;
                }
                println!("    -> VdSetGraphicsInterruptCallback: Callback=0x{:08X}, Context=0x{:08X}",
                    self.graphics_interrupt_callback, self.graphics_interrupt_context);
                0
            }

            // 0x01D9: VdSetSystemCommandBufferGpuIdentifierAddress (pGpuIdent)
            0x01D9 => {
                if let Some(&ptr) = args.first() {
                    self.gpu_identifier_ptr = ptr;
                    if ptr != 0 {
                        let _ = bus.write_u32(ptr, 0x0000_0001);
                    }
                }
                0
            }

            // 0x01DF: KiApcNormalRoutineNop
            0x01DF => 0,

            // 0x01E7: XInputdWriteState | 0x01E8: XInputdNotify
            0x01E7 | 0x01E8 => 0,

            // 0x01F3..0x0200: XAudio Handlers
            0x01F3 | 0x01F4 | 0x01F5 | 0x01F7 | 0x01F8 | 0x01FA | 0x01FB | 0x01FC | 0x0200 => 0,

            // 0x021C: XUsbcamSetConfig | 0x021D: XUsbcamGetState | 0x0221: XUsbcamGetView
            0x021C | 0x021D | 0x0221 => 0,

            // 0x0224: XMACreateContext | 0x0226: XMAReleaseContext
            0x0224 => 0x0000_3000,
            0x0226 => 0,

            // 0x0244: XeKeysGetKey (keyIndex, pBuffer)
            0x0244 => {
                if let Some(&p_buf) = args.get(1) {
                    if p_buf != 0 {
                        for off in (0..16).step_by(4) {
                            let _ = bus.write_u32(p_buf + off, 0);
                        }
                    }
                }
                0
            }

            // 0x0246: XeKeysGetConsoleID (pConsoleId)
            0x0246 => {
                if let Some(&p_id) = args.first() {
                    if p_id != 0 {
                        let _ = bus.write_u32(p_id, 0x0020_11B4);
                        let _ = bus.write_u32(p_id + 4, 0x0000_0001);
                    }
                }
                0
            }

            // 0x025B: VdSwap (Presentation / Frame Swap)
            0x025B => {
                self.swap_count += 1;
                if let Some(&fb) = args.first() {
                    if fb != 0 {
                        self.last_swap_frontbuffer = fb;
                    }
                }
                0
            }

            // 0x0266: KeCertMonitorData
            0x0266 => 0,

            // 0x0269: VdRetrainEDRAM | 0x026A: VdRetrainEDRAMWorker
            0x0269 | 0x026A => 0,

            // 0x0270: VdEnumerateVideoModes | 0x027E: VdGetDisplayDiscoveryData
            0x0270 | 0x027E => 0,

            // 0x02BD: HalRegisterXamPowerDownCallback
            0x02BD => 0,

            // 0x02E1: RmcDeviceRequest
            0x02E1 => 0,

            // 0x0334: EtxProducerRegister
            0x0334 => 0,

            // 0x0379: VdSetStudioRGBMode
            0x0379 => 0,

            _ => {
                println!("    [HLE KERNEL] Unhandled xboxkrnl.exe ordinal 0x{:04X} ({})", ordinal, ordinal);
                0
            }
        }
    }

    fn dispatch_xam(
        &mut self,
        ordinal: u16,
        _func_name: &str,
        args: &[u64],
        bus: &mut MemoryBus,
    ) -> u64 {
        match ordinal {
            // 0x0001: NetDll_WSAStartup (caller, version, pWsaData)
            0x0001 => {
                if let Some(&p_wsa) = args.get(2) {
                    if p_wsa != 0 {
                        let _ = bus.write_u16(p_wsa + 0, 0x0202); // wVersion 2.2
                        let _ = bus.write_u16(p_wsa + 2, 0x0202); // wHighVersion 2.2
                    }
                }
                0 // SUCCESS
            }

            // 0x0002: NetDll_WSACleanup
            0x0002 => 0,

            // 0x0003: NetDll_socket
            0x0003 => {
                let s = self.next_handle;
                self.next_handle += 4;
                s as u64
            }

            // 0x0004..0x0018: Network calls
            0x0004 | 0x0005 | 0x0006 | 0x0007 | 0x0009 | 0x000A | 0x000B | 0x000D | 0x000E
            | 0x000F | 0x0010 | 0x0011 | 0x0012 | 0x0013 | 0x0014 | 0x0016 | 0x0017 | 0x0018
            | 0x006C | 0x006D | 0x006E | 0x006F => 0,

            // 0x005F: XamAppRegister (appId, pAppObj)
            0x005F => {
                self.xam_app_registered = true;
                self.xam_app_id = args.first().copied().unwrap_or(0) as u32;
                println!("    -> XamAppRegister: Registered Dashboard XAM App ID=0x{:X}", self.xam_app_id);
                1 // Registered successfully
            }

            // 0x007D: XamAppAttach
            0x007D => 0,

            // 0x0190: XamInputGetCapabilities (userIndex, flags, pCaps)
            0x0190 => {
                if let Some(&p_caps) = args.get(2) {
                    if p_caps != 0 {
                        for off in (0..20).step_by(4) {
                            let _ = bus.write_u32(p_caps + off, 0);
                        }
                        let _ = bus.write_u8(p_caps + 0, 1); // XINPUT_DEVTYPE_GAMEPAD
                        let _ = bus.write_u8(p_caps + 1, 1); // XINPUT_DEVSUBTYPE_GAMEPAD
                    }
                }
                0 // ERROR_SUCCESS
            }

            // 0x0191: XamInputGetState (userIndex, flags, pState)
            0x0191 => {
                let user_index = args.first().copied().unwrap_or(0);
                if user_index == 0 {
                    if let Some(&p_state) = args.get(2) {
                        if p_state != 0 {
                            let _ = bus.write_u32(p_state + 0, self.gamepad_packet_number);
                            let _ = bus.write_u16(p_state + 4, self.gamepad_state.w_buttons);
                            let _ = bus.write_u8(p_state + 6, self.gamepad_state.b_left_trigger);
                            let _ = bus.write_u8(p_state + 7, self.gamepad_state.b_right_trigger);
                            let _ = bus.write_u16(p_state + 8, self.gamepad_state.s_thumb_lx as u16);
                            let _ = bus.write_u16(p_state + 10, self.gamepad_state.s_thumb_ly as u16);
                            let _ = bus.write_u16(p_state + 12, self.gamepad_state.s_thumb_rx as u16);
                            let _ = bus.write_u16(p_state + 14, self.gamepad_state.s_thumb_ry as u16);
                        }
                    }
                    0 // ERROR_SUCCESS
                } else {
                    0x0000_0467 // ERROR_DEVICE_NOT_CONNECTED (1167)
                }
            }

            // 0x0192: XamInputSetState
            0x0192 => 0,

            // 0x0196: XamEnableSystemAppInput
            0x0196 => 0,

            // 0x0198: XamInputGetKeystrokeEx
            0x0198 => 0x0000_0490, // ERROR_EMPTY

            // 0x01A8: XamLoaderGetLaunchData (pLaunchData, cbLaunchData)
            0x01A8 => {
                if let (Some(&p_data), Some(&cb)) = (args.first(), args.get(1)) {
                    if p_data != 0 && cb > 0 {
                        for i in 0..cb.min(4096) {
                            let _ = bus.write_u8(p_data + i, 0);
                        }
                    }
                }
                println!("    -> XamLoaderGetLaunchData: Returned clean boot launch state (STATUS_SUCCESS)");
                0 // STATUS_SUCCESS
            }

            // 0x01AA: XamLoaderGetDvdTrayState
            0x01AA => 0, // Tray closed

            // 0x01B1: XamTaskCloseHandle
            0x01B1 => 0,

            // 0x01B9: XamInputSendStayAliveRequest
            0x01B9 => 0,

            // 0x01C5: XamGetDefaultSystemImage (phImage)
            0x01C5 => {
                if let Some(&p_img) = args.first() {
                    if p_img != 0 {
                        let _ = bus.write_u32(p_img, 0x0000_0001);
                    }
                }
                0
            }

            // 0x01C7: XamSetPowerMode
            0x01C7 => 0,

            // 0x01CC: XamPartyGetUserList
            0x01CC => 0,

            // 0x01CF: XamGetCurrentTitleId
            0x01CF => 0xFFFE_07D1, // Dashboard Title ID

            // 0x01D1: XamAllocHeapFreeSpace
            0x01D1 => 64 * 1024 * 1024,

            // 0x01D2: XamSetDashContextEx
            0x01D2 => 0,

            // 0x01EA: XamAlloc (size)
            // 0x01EB: XamAllocEx (size, ...)
            0x01EA | 0x01EB => {
                let requested_size = args.first().copied().unwrap_or(64) as u32;
                let size = if requested_size == 0 { 64 } else { requested_size };
                let aligned_size = (size + 31) & !31;
                let ptr = self.valloc_bump;
                self.valloc_bump += aligned_size as u64;

                // valloc_bump hands out fresh, never-reused addresses whose backing pages read as zero,
                // so zeroing is only needed for tiny blocks; never loop up to 4 GB on a bogus size.
                for i in 0..aligned_size.min(0x10000) {
                    let _ = bus.write_u8(ptr + i as u64, 0);
                }

                println!("    -> XamAlloc/Ex: Allocated 0x{:X} bytes at guest pointer 0x{:08X}", size, ptr);
                ptr
            }

            // 0x01EC: XamFree
            0x01EC => 0,

            // 0x01ED: XamAllocSize
            0x01ED => 64,

            // 0x01F4: XMsgInProcessCall
            0x01F4 => 0,

            // 0x01F5: XamGetExecutionId / XMsgCompleteIORequest
            // Writes genuine Dashboard XEX_EXECUTION_ID (24 bytes)
            0x01F5 => {
                if let Some(&p_exec) = args.first() {
                    if p_exec != 0 {
                        let _ = bus.write_u32(p_exec + 0, 0x0000_0000); // media_id
                        let _ = bus.write_u32(p_exec + 4, 0x2044_9700); // version (2.0.17559.0)
                        let _ = bus.write_u32(p_exec + 8, 0x2044_9700); // base_version
                        let _ = bus.write_u32(p_exec + 12, 0xFFFE_07D1); // title_id
                        let _ = bus.write_u8(p_exec + 16, 1);           // platform (Xbox 360)
                        let _ = bus.write_u8(p_exec + 17, 0);           // executable_table
                        let _ = bus.write_u8(p_exec + 18, 1);           // disc_number
                        let _ = bus.write_u8(p_exec + 19, 1);           // disc_count
                    }
                }
                0
            }

            // 0x020A: XamUserGetXUID (userIndex, flags, pXuid)
            0x020A => {
                if let Some(&p_xuid) = args.get(2) {
                    if p_xuid != 0 {
                        let _ = bus.write_u32(p_xuid + 0, 0x0009_0000);
                        let _ = bus.write_u32(p_xuid + 4, 0x0000_0001);
                    }
                }
                0
            }

            // 0x020B: XamUserLogon
            0x020B => 0,

            // 0x020C: XamUserGetGamerTag (userIndex, pBuffer, cchBuffer)
            0x020C => {
                if let (Some(&p_buf), Some(&cch)) = (args.get(1), args.get(2)) {
                    if p_buf != 0 && cch >= 8 {
                        let tag = b"Xbox360\0";
                        for (i, &b) in tag.iter().enumerate() {
                            let _ = bus.write_u8(p_buf + i as u64, b);
                        }
                    }
                }
                0
            }


            // 0x0210: XamUserGetSigninState (userIndex)
            0x0210 => self.signin_state as u64, // 1 = Signed in locally

            // 0x0211: XamUserGetIndexFromXUID (xuid, pUserIndex)
            0x0211 => {
                if let Some(&p_idx) = args.get(1) {
                    if p_idx != 0 {
                        let _ = bus.write_u32(p_idx, 0);
                    }
                }
                0
            }

            // 0x021B: XamUserGetMembershipTier
            0x021B => 6, // Gold Membership

            // 0x021C: XamUserGetUserFlags (userIndex, flags, pFlags*) - report "signed in locally, no Live"
            0x021C => {
                if let Some(&p) = args.get(3) { if p != 0 { let _ = bus.write_u32(p, 0); } }
                0
            }

            // 0x03C3: XamUserGetUserTenure (userIndex, pTenureMonths*, pAccountYears*, pLiveYears*) - brand-new local profile
            // 0x03C4: XamUserGetSubscriptionType (userIndex, pType*, pExpiry*) - none (offline)
            0x03C3 | 0x03C4 => {
                for k in 1..=3usize {
                    if let Some(&p) = args.get(k) { if p != 0 && (p & 3) == 0 { let _ = bus.write_u32(p, 0); } }
                }
                0
            }

            // 0x0219: XamUserReadProfileSettings (titleId, flags, userCount, pXuids, settingCount, pIds, pcbResults, pResults, overlapped)
            // Offline: no profile settings exist. Report an empty, well-formed XUSER_READ_PROFILE_SETTING_RESULT
            // (settings_count = 0, settings ptr = NULL) so callers do not wait on garbage.
            0x0219 => {
                let pcb = args.get(6).copied().unwrap_or(0);
                let p_res = args.get(7).copied().unwrap_or(0);
                if p_res != 0 {
                    for off in (0..0x10u64).step_by(4) { let _ = bus.write_u32(p_res + off, 0); }
                }
                if pcb != 0 { let _ = bus.write_u32(pcb, 0x10); }
                0 // ERROR_SUCCESS
            }

            // 0x01A3: XamLoaderGetMediaInfo (pMediaInfo*, pFlags*) - no disc inserted
            0x01A3 => {
                for k in 0..=1usize {
                    if let Some(&p) = args.get(k) { if p != 0 && (p & 3) == 0 { let _ = bus.write_u32(p, 0); } }
                }
                0
            }

            // 0x070D: XamQueryLiveHiveW (name, output, size, type) - the Live registry hive has no entries offline
            0x070D => 0x0000_0490, // ERROR_NOT_FOUND

            // 0x0024 NetDll_WSAStartupEx / 0x0033 NetDll_XNetStartup: success
            0x0024 | 0x0033 => 0,
            // 0x00C9 NetDll_XHttpStartup / 0x00CA NetDll_XHttpShutdown return BOOL (TRUE). The dash treats 0 as failure
            // and then reads GetLastError(), skipping the rest of its service initialisation (this left the singleton
            // at 0x92A7C23C unset and led to the call through NULL at frame 173).
            0x00C9 | 0x00CA => 1,
            // 0x004B NetDll_XNetGetEthernetLinkStatus: no cable
            0x004B => 0,

            // 0x0220: XamUserCheckPrivilege (userIndex, privilegeType, pGranted)
            0x0220 => {
                if let Some(&p_granted) = args.get(2) {
                    if p_granted != 0 {
                        let _ = bus.write_u32(p_granted, 1); // 1 = TRUE (Granted)
                    }
                }
                0 // ERROR_SUCCESS
            }

            // 0x0221: XamUserGetSigninInfo (userIndex, flags, pSigninInfo)
            0x0221 => {
                if let Some(&p_info) = args.get(2) {
                    if p_info != 0 {
                        let _ = bus.write_u32(p_info + 0, 0x0009_0000); // xuid high
                        let _ = bus.write_u32(p_info + 4, 0x0000_0001); // xuid low
                        let _ = bus.write_u32(p_info + 8, 0);           // dwInfoFlags
                        let _ = bus.write_u32(p_info + 12, self.signin_state as u32); // 1 = Signed in locally
                        let _ = bus.write_u32(p_info + 16, 0);          // guest number
                        let _ = bus.write_u32(p_info + 20, 0);          // sponsor user index
                    }
                }
                0 // ERROR_SUCCESS
            }

            // 0x0222: XamUserGetName
            0x0222 => {
                if let (Some(&p_buf), Some(&cch)) = (args.get(1), args.get(2)) {
                    if p_buf != 0 && cch >= 8 {
                        let tag = b"Xbox360\0";
                        for (i, &b) in tag.iter().enumerate() {
                            let _ = bus.write_u8(p_buf + i as u64, b);
                        }
                    }
                }
                0
            }

            // 0x0223: XamUserAreUsersFriends | 0x0225: XamUserReadProfileSettings | 0x0226: XamUserWriteProfileSettings | 0x0228: XamUserReadStats
            0x0223 | 0x0225 | 0x0226 | 0x0228 => 0,

            // 0x0227: XamUserCreateAchievementEnumerator
            0x0227 => {
                if let Some(&p_h) = args.get(4) {
                    if p_h != 0 {
                        let h = self.next_handle;
                        self.next_handle += 4;
                        let _ = bus.write_u32(p_h, h);
                    }
                }
                0
            }

            // 0x0230: XamProfileCreate | 0x0234: XamProfileGetCreationStatus
            0x0230 | 0x0234 => 0,

            // 0x0246: XamAppUnloadStack
            0x0246 => 0,

            // 0x0250: XamEnumerate
            0x0250 => 0x0000_0103, // STATUS_NO_MORE_FILES

            // 0x0259: XamContentCreateEx
            0x0259 => 0,

            // 0x0269: XamContentOpenFile | 0x0278: XamContentOpenFileInternal | 0x027B: XamContentOpenPackageFile
            0x0269 | 0x0278 | 0x027B => {
                let p_path = args.get(2).copied().unwrap_or(0);
                let p_handle = args.get(4).copied().unwrap_or(0);
                if let Some(path) = read_c_string(p_path, bus) {
                    println!("    [VFS] XamContentOpenFile: '{}'", path);
                    if let Some(data) = self.find_vfs_file(&path) {
                        let h = self.next_handle;
                        self.next_handle += 4;
                        self.vfs_handles.insert(h, VfsFileHandle {
                            handle: h,
                            name: path.clone(),
                            data,
                            position: 0,
                            is_directory: false,
                        });
                        if p_handle != 0 {
                            let _ = bus.write_u32(p_handle, h);
                        }
                        0
                    } else {
                        0xC000_0034
                    }
                } else {
                    0xC000_000D
                }
            }

            // 0x0276: XamContentResolveInternal (userIndex, flags, pPathBuffer, cchBuffer, ...)
            0x0276 => {
                if let Some(&p_buf) = args.get(2) {
                    if p_buf != 0 {
                        let path = b"game:\\\0";
                        for (i, &b) in path.iter().enumerate() {
                            let _ = bus.write_u8(p_buf + i as u64, b);
                        }
                    }
                }
                println!("    -> XamContentResolveInternal: Resolved content root path to 'game:\\'");
                0 // ERROR_SUCCESS
            }

            // 0x028A: XNotifyCreateListener
            0x028A => {
                let h = self.next_handle;
                self.next_handle += 4;
                h as u64
            }

            // 0x028B: XNotifyGetNext (hListener, flags, pId, pParam)
            0x028B => {
                if let Some(&p_id) = args.get(2) {
                    if p_id != 0 {
                        let _ = bus.write_u32(p_id, 0); // No pending notifications
                    }
                }
                0
            }

            // 0x028C | 0x028D | 0x0290 | 0x0292 | 0x02A0 | 0x02A1: Notifications and Dialogs
            0x028C | 0x028D | 0x0290 | 0x0292 | 0x02A0 | 0x02A1 => 0,

            // 0x02B4: XamCacheOpenFile
            0x02B4 => {
                let h = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.get(3) {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, h);
                    }
                }
                0
            }

            // 0x02B5: XamCacheCloseFile
            0x02B5 => 0,

            // 0x02B6: XamGetCachedTitleName
            0x02B6 => {
                if let Some(&p_buf) = args.get(1) {
                    if p_buf != 0 {
                        let title = "Xbox 360 Dashboard\0";
                        for (i, c) in title.encode_utf16().enumerate() {
                            let _ = bus.write_u16(p_buf + (i * 2) as u64, c);
                        }
                    }
                }
                0
            }

            // 0x02B9: XamGetCachedGamerTagW
            0x02B9 => {
                if let Some(&p_buf) = args.get(1) {
                    if p_buf != 0 {
                        let tag = "Xbox360\0";
                        for (i, c) in tag.encode_utf16().enumerate() {
                            let _ = bus.write_u16(p_buf + (i * 2) as u64, c);
                        }
                    }
                }
                0
            }

            // 0x02BC..0x031D, 0x0492, 0x04A9: System UI Blading and Dialog Services
            0x02BC | 0x02BE | 0x02BF | 0x02D1 | 0x02E3 | 0x02E4 | 0x02F0 | 0x02F8 | 0x02FC | 0x030A | 0x031D | 0x0492 | 0x04A9 => 0,

            // 0x0324: XamVoiceCreate | 0x0325: XamVoiceHeadsetPresent | 0x0330: XamContentGetMarketplaceCounts
            0x0324 | 0x0325 | 0x0330 => 0,

            // 0x03CB: XGetAVPack
            0x03CB => 6, // XAVPACK_HDMI

            // 0x03D1: XGetVideoMode
            0x03D1 => {
                if let Some(&p_mode) = args.first() {
                    if p_mode != 0 {
                        let _ = bus.write_u32(p_mode + 0, 1280);
                        let _ = bus.write_u32(p_mode + 4, 720);
                        let _ = bus.write_u32(p_mode + 8, 0);
                        let _ = bus.write_u32(p_mode + 12, 1);
                        let _ = bus.write_u32(p_mode + 16, 60);
                    }
                }
                0
            }

            // 0x03D2: XVideoSetMode
            0x03D2 => 0,

            // 0x0417: SetFilePointer (hFile, lDistanceToMove, lpDistanceToMoveHigh, dwMoveMethod)
            0x0417 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let dist = args.get(1).copied().unwrap_or(0) as i32;
                let method = args.get(3).copied().unwrap_or(0) as u32;
                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    let new_pos = match method {
                        0 => dist.max(0) as usize, // FILE_BEGIN
                        1 => (file.position as i64 + dist as i64).max(0) as usize, // FILE_CURRENT
                        2 => (file.data.len() as i64 + dist as i64).max(0) as usize, // FILE_END
                        _ => file.position,
                    };
                    file.position = new_pos;
                    file.position as u64
                } else {
                    0xFFFF_FFFF
                }
            }

            // 0x041C: ReadFile (hFile, lpBuffer, nNumberOfBytesToRead, lpNumberOfBytesRead, lpOverlapped)
            0x041C => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_buf = args.get(1).copied().unwrap_or(0);
                let len = args.get(2).copied().unwrap_or(0) as usize;
                let p_read = args.get(3).copied().unwrap_or(0);
                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    let avail = file.data.len().saturating_sub(file.position);
                    let to_read = std::cmp::min(len, avail);
                    if to_read > 0 && p_buf != 0 {
                        let slice = &file.data[file.position..file.position + to_read];
                        let _ = bus.load_bytes(p_buf, slice);
                        file.position += to_read;
                    }
                    if p_read != 0 {
                        let _ = bus.write_u32(p_read, to_read as u32);
                    }
                    1 // TRUE
                } else {
                    0 // FALSE
                }
            }

            // 0x0427: GetFileSize (hFile, lpFileSizeHigh)
            0x0427 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_high = args.get(1).copied().unwrap_or(0);
                if let Some(file) = self.vfs_handles.get(&h) {
                    if p_high != 0 {
                        let _ = bus.write_u32(p_high, 0);
                    }
                    file.data.len() as u64
                } else {
                    0xFFFF_FFFF
                }
            }

            // 0x0447: CreateFileA (lpFileName, dwDesiredAccess, dwShareMode, lpSecurityAttributes, dwCreationDisposition, ...)
            0x0447 => {
                let p_name = args.first().copied().unwrap_or(0);
                if let Some(path) = read_c_string(p_name, bus) {
                    println!("    [VFS] CreateFileA: '{}'", path);
                    if let Some(data) = self.find_vfs_file(&path) {
                        let h = self.next_handle;
                        self.next_handle += 4;
                        self.vfs_handles.insert(h, VfsFileHandle {
                            handle: h,
                            name: path.clone(),
                            data,
                            position: 0,
                            is_directory: false,
                        });
                        h as u64
                    } else {
                        0xFFFF_FFFF_FFFF_FFFF // INVALID_HANDLE_VALUE
                    }
                } else {
                    0xFFFF_FFFF_FFFF_FFFF
                }
            }

            // 0x04C1: XamFitnessClearBodyProfileRecords
            0x04C1 => 0,

            // 0x04E4: XamBackgroundDownloadNetworkStorageRegisterChangeCallback
            0x04E4 => 0,

            // 0x050B: XamGetConsoleFriendlyName
            0x050B => {
                if let Some(&p_buf) = args.first() {
                    if p_buf != 0 {
                        let name = "Xbox 360\0";
                        for (i, c) in name.encode_utf16().enumerate() {
                            let _ = bus.write_u16(p_buf + (i * 2) as u64, c);
                        }
                    }
                }
                0
            }

            // 0x01CB (459): XamGetOnlineSchema() -> const struct { const void *data; uint32_t size; } *
            // The dash passes the returned pointer straight into its schema parser (0x9224AA48), which fails with
            // ERROR_INVALID_PARAMETER (0x57) on NULL and aborts the whole service initialisation. We return a minimal,
            // valid schema: a 44-byte header (all table counts 0, flags clear) whose total size is stored at +20.
            0x01CB => {
                if self.online_schema_ptr == 0 {
                    const SCHEMA_SIZE: u32 = 0x2C;
                    let blob = self.valloc_bump;
                    self.valloc_bump += 0x100;
                    let st = self.valloc_bump;
                    self.valloc_bump += 0x10;
                    for i in 0..0x100u64 { let _ = bus.write_u8(blob + i, 0); }
                    let _ = bus.write_u32(blob + 20, SCHEMA_SIZE); // total schema size
                    let _ = bus.write_u32(st, blob as u32);       // data*
                    let _ = bus.write_u32(st + 4, SCHEMA_SIZE);   // size
                    self.online_schema_ptr = st;
                    println!("    -> XamGetOnlineSchema: built minimal schema struct @0x{:08X} (data @0x{:08X}, size 0x{:X})", st, blob, SCHEMA_SIZE);
                }
                self.online_schema_ptr
            }

            // 0x050C: XamSetConsoleFriendlyName
            0x050C => 0,

            // 0x058B: XamGetSystemVersion
            0x058B => 0x2044_9700, // Version 2.0.17559.0

            // 0x0646: XamContentCreateEnumerator
            0x0646 => {
                let h = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_handle) = args.get(6) {
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h);
                    }
                }
                if let Some(&p_cb) = args.get(5) {
                    if p_cb != 0 {
                        let _ = bus.write_u32(p_cb, 0x1000);
                    }
                }
                println!("    -> XamContentCreateEnumerator: Allocated enumerator handle 0x{:X}", h);
                0 // ERROR_SUCCESS
            }

            // 0x0647: XamContentGetEnumeratorAttributes | 0x0648: XamContentCloseEnumerator | 0x0649: XamContentFlush
            0x0647 | 0x0648 | 0x0649 => 0,

            // 0x0650: XdfInitialize
            0x0650 => 0,

            // 0x076F: XamGetStagingMode
            0x076F => 0,

            // 0x07DC: XamGetDashBackstackData
            0x07DC => 0,

            // 0x09C4: XamBackgroundDownloadSetMode
            0x09C4 => 0,

            // 0x0B43: XamSetCountry
            0x0B43 => 0,

            // 0x0B45: XamLoaderRebootToServerDash
            0x0B45 => 0,

            _ => {
                println!("    [HLE XAM] Unhandled xam.xex ordinal 0x{:04X} ({})", ordinal, ordinal);
                0
            }
        }
    }
}
