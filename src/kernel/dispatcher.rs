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
    /// The thread's X_KTHREAD in guest memory.
    pub kthread: u32,
    pub start_routine: u64,
    pub start_context: u64,
    pub stack_size: u32,
    pub creation_flags: u32,
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
    /// Handle table and kernel objects (kernel/objects.rs).
    pub objects: super::objects::ObjectTable,
    /// Handle of the thread making the current call (NtCurrentThread); 0 when not running on a kernel thread.
    pub current_thread: u32,
    /// Wait the calling thread must be parked with (taken by the frame loop after the call), see kernel/scheduler.rs.
    pub pending_wait: Option<super::scheduler::WaitRequest>,
    /// X_KTHREAD of the calling thread (its KPCR's current thread), for ownership: mutants, critical sections.
    pub current_kthread: u32,
    /// Set kernel timers by KTIMER address (kernel/timers.rs).
    pub timers: super::timers::TimerQueue,
    /// Queued notifications of I/O completion ports, by the port's guest header.
    pub io_completions: HashMap<u32, std::collections::VecDeque<super::timers::IoNotification>>,
    /// XAM notification listeners by handle (kernel/notify.rs).
    pub notifications: super::notify::Notifications,
    /// Thread handles by thread id (ObLookupThreadByThreadId).
    pub thread_by_id: HashMap<u32, u32>,
    pub tls_slots: HashMap<u32, u64>,
    pub next_tls_index: u32,
    /// Bump pointer for guest virtual allocations (NtAllocateVirtualMemory + HLE helper buffers).
    /// Lives in the bus virtual-alloc window (0x40000000..0x8FFFFFFF), which has private backing pages.
    pub valloc_bump: u64,
    /// Bump pointer for physical allocations (MmAllocatePhysicalMemoryEx: ring buffer, framebuffers, GPU data).
    /// These keep `addr & 0x1FFFFFFF` as their physical address, so they must not overlap the dash image (phys 0x12000000).
    pub phys_bump: u64,
    /// Guest address of the cached `{ data*, size }` struct returned by XamGetOnlineSchema (0 = not built yet)
    pub online_schema_ptr: u64,
    /// xorshift64 state for XeCryptRandom
    pub rng_state: u64,
    pub signin_state: u32,
    /// XamLoaderRegisterLaunchRequestCallback / XamContentRegisterChangeCallback routines (stored, never called:
    /// no launch requests or content changes happen here).
    pub launch_request_callback: u32,
    pub content_change_callback: u32,
    /// XamSetDashContext value.
    pub dash_context: u32,
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
    pub vfs_files: HashMap<String, Vec<u8>>,
    /// Loaded modules and their loader data entries (XexGetModuleHandle / XexGetModuleSection).
    pub modules: super::modules::ModuleTable,
    /// DPCs inserted with KeInsertQueueDpc and not removed (KeRemoveQueueDpc).
    pub queued_dpcs: std::collections::HashSet<u32>,
    /// DPC routine the calling thread must run before returning from KeInsertQueueDpc (taken by the frame loop).
    pub pending_dpc_call: Option<super::dpc::DpcCall>,
    /// Set by XamLoaderTerminateTitle: the title exits to the dashboard, the frame loop stops emulation.
    pub title_terminated: bool,
    /// Video mode reported by VdQueryVideoMode.
    pub video: super::video::VideoConfig,
    pub vfs_handles: HashMap<u32, VfsFileHandle>,
    pub host: Box<dyn crate::host::Host>,
}

impl KernelDispatcher {
    pub fn new() -> Self {
        Self {
            next_thread_id: 1,
            objects: Default::default(),
            current_thread: 0,
            pending_wait: None,
            current_kthread: 0,
            thread_by_id: HashMap::new(),
            notifications: Default::default(),
            timers: Default::default(),
            io_completions: HashMap::new(),
            tls_slots: HashMap::new(),
            next_tls_index: 1,
            valloc_bump: 0x4000_0000,
            phys_bump: 0x9010_0000,
            online_schema_ptr: 0,
            rng_state: 0x9E37_79B9_7F4A_7C15,
            signin_state: 1,
            dash_context: 0,
            launch_request_callback: 0,
            content_change_callback: 0,
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
            vfs_files: HashMap::new(),
            modules: super::modules::ModuleTable::default(),
            queued_dpcs: std::collections::HashSet::new(),
            pending_dpc_call: None,
            title_terminated: false,
            video: super::video::VideoConfig::default(),
            vfs_handles: HashMap::new(),
            host: crate::host::default_host(),
        }
    }

    /// A handle to a new object without a guest object (files, sockets, listeners, enumerators, ...).
    pub fn new_handle(&mut self, kind: super::objects::ObjectKind) -> u32 {
        self.objects.create(super::objects::KernelObject { kind, guest_ptr: 0 })
    }

    /// SystemHeapAlloc: zeroed guest memory for kernel objects, 16-byte aligned, from the virtual-alloc window.
    pub fn alloc_system(&mut self, bus: &mut MemoryBus, size: u32) -> u64 {
        let ptr = (self.valloc_bump + 15) & !15;
        self.valloc_bump = ptr + size as u64;
        for off in (0..size as u64).step_by(4) {
            let _ = bus.write_u32(ptr + off, 0);
        }
        ptr
    }

    /// XObject::CreateNative: X_OBJECT_HEADER + a dispatcher object body; returns the body's address.
    pub fn create_native(&mut self, bus: &mut MemoryBus, body_size: u32) -> u64 {
        self.alloc_system(bus, super::objects::OBJECT_HEADER_SIZE + body_size) + super::objects::OBJECT_HEADER_SIZE as u64
    }

    /// A thread object: X_KTHREAD in guest memory (XThread::InitializeGuestObject) and its handle.
    pub fn create_thread_object(&mut self, bus: &mut MemoryBus, init: &super::objects::KThreadInit) -> (u32, u32) {
        let kthread = self.alloc_system(bus, super::objects::KTHREAD_SIZE);
        super::objects::init_kthread(bus, kthread, init);
        let handle = self.objects.create(super::objects::KernelObject { kind: super::objects::ObjectKind::Thread, guest_ptr: kthread as u32 });
        self.thread_by_id.insert(init.thread_id, handle);
        (handle, kthread as u32)
    }

    /// xeXamNotifyCreateListener: a listener object whose wait handle is a notification KEVENT, registered for the
    /// startup notifications.
    pub fn create_notify_listener(&mut self, bus: &mut MemoryBus, mask: u64, max_version: u32) -> u32 {
        use super::objects::{init_header, stash_handle, KernelObject, ObjectKind, EVENT_NOTIFICATION_OBJECT, KEVENT_SIZE};
        let event = self.create_native(bus, KEVENT_SIZE);
        init_header(bus, event, EVENT_NOTIFICATION_OBJECT, KEVENT_SIZE, 0);
        let handle = self.objects.create(KernelObject { kind: ObjectKind::NotifyListener, guest_ptr: event as u32 });
        stash_handle(bus, event, handle);
        let listener = super::notify::Listener { mask, max_version: max_version.min(10), event: event as u32, queue: Default::default() };
        // the dashboard's title id (XamGetCurrentTitleId)
        self.notifications.register(bus, handle, listener, true, crate::diag::config().startup_notifications);
        handle
    }

    /// KernelState::CompleteOverlappedImmediate: XAM_OVERLAPPED result, extended error and length (0 on success,
    /// 0xFFFFFFFF otherwise), context = the calling thread, and its event set when there is no completion routine
    /// (completion routines are APCs, not emulated yet).
    pub fn complete_overlapped_immediate(&mut self, bus: &mut MemoryBus, overlapped: u64, result: u32) {
        let _ = bus.write_u32(overlapped, result);
        let _ = bus.write_u32(overlapped + 4, if result == 0 { 0 } else { 0xFFFF_FFFF });
        let _ = bus.write_u32(overlapped + 8, self.current_thread);
        let _ = bus.write_u32(overlapped + 0x18, result);
        let event = bus.read_u32(overlapped + 0xC).unwrap_or(0);
        let routine = bus.read_u32(overlapped + 0x10).unwrap_or(0);
        if event != 0 && routine == 0 {
            if let Ok(ev) = self.handle_object(event as u64, super::objects::ObjectKind::Event) {
                super::objects::set_event(bus, ev);
            }
        }
    }

    /// A NUL-terminated big-endian UTF-16 guest string (at most 512 units).
    pub fn read_utf16(bus: &MemoryBus, ptr: u64) -> String {
        if ptr == 0 {
            return String::new();
        }
        let units: Vec<u16> = (0..512u64).map(|i| bus.read_u16(ptr + 2 * i).unwrap_or(0)).take_while(|&u| u != 0).collect();
        String::from_utf16_lossy(&units)
    }

    /// string_util::copy_and_swap_truncating: at most `count` UTF-16 units (big-endian), the last one a terminator.
    pub fn write_utf16_truncating(bus: &mut MemoryBus, ptr: u64, text: &str, count: usize) {
        if ptr == 0 || count == 0 {
            return;
        }
        let units: Vec<u16> = text.encode_utf16().take(count - 1).collect();
        for (i, u) in units.iter().enumerate() {
            let _ = bus.write_u16(ptr + 2 * i as u64, *u);
        }
        let _ = bus.write_u16(ptr + 2 * units.len() as u64, 0);
    }

    /// Kernel variables exported to the guest (Xenia ExportResolver::SetVariableMapping), as (ordinal, address):
    /// the video ones from RegisterVideoExports - VdGlobalDevice (points to a 40-byte block), VdGlobalXamDevice (0),
    /// VdGpuClockInMHz (500) and VdHSIOCalibrationLock (a critical section, spin count 10000). Other kernel
    /// variables are not exported yet.
    pub fn register_variable_exports(&mut self, bus: &mut MemoryBus) -> Vec<(u16, u32)> {
        use super::video::{VD_GLOBAL_DEVICE, VD_GLOBAL_XAM_DEVICE, VD_GPU_CLOCK_IN_MHZ, VD_HSIO_CALIBRATION_LOCK};
        let device_block = self.alloc_system(bus, 40) as u32;
        let _ = bus.write_u32(VD_GLOBAL_DEVICE.1 as u64, device_block);
        let _ = bus.write_u32(VD_GLOBAL_XAM_DEVICE.1 as u64, 0);
        let _ = bus.write_u32(VD_GPU_CLOCK_IN_MHZ.1 as u64, 500);
        super::objects::init_critical_section(bus, VD_HSIO_CALIBRATION_LOCK.1 as u64, ((10_000u32 + 255) >> 8) as u8);
        vec![VD_GLOBAL_DEVICE, VD_GLOBAL_XAM_DEVICE, VD_GPU_CLOCK_IN_MHZ, VD_HSIO_CALIBRATION_LOCK]
    }

    /// Starts a wait for the calling thread: it completes at once if it can (returning its status), otherwise the
    /// thread is parked by the frame loop (`pending_wait`) and gets the status when the wait completes.
    pub fn begin_wait(&mut self, bus: &mut MemoryBus, wait: super::scheduler::WaitRequest) -> u64 {
        let now = bus.timebase.now();
        match super::scheduler::evaluate(bus, &wait, now) {
            super::scheduler::WaitOutcome::Done(status) => self.complete_wait(bus, &wait, status),
            super::scheduler::WaitOutcome::Pending => {
                self.pending_wait = Some(wait);
                0
            }
        }
    }

    /// Kernel-side part of a completed wait (after the scheduler acquired its objects); returns the final status.
    pub fn complete_wait(&mut self, bus: &mut MemoryBus, wait: &super::scheduler::WaitRequest, status: u64) -> u64 {
        if let super::scheduler::WakeAction::RemoveIoCompletion { port, key_ptr, apc_ptr, iosb_ptr } = wait.on_wake {
            if status == super::scheduler::STATUS_WAIT_0 {
                let n = self.io_completions.get_mut(&port).and_then(|q| q.pop_front()).unwrap_or_default();
                if key_ptr != 0 {
                    let _ = bus.write_u32(key_ptr as u64, n.key_context);
                }
                if apc_ptr != 0 {
                    let _ = bus.write_u32(apc_ptr as u64, n.apc_context);
                }
                if iosb_ptr != 0 {
                    let _ = bus.write_u32(iosb_ptr as u64, n.status);
                    let _ = bus.write_u32(iosb_ptr as u64 + 4, n.num_bytes);
                }
                return 0;
            }
        }
        status
    }

    /// Deadline of a wait's timeout pointer: none = wait forever, 0 = only poll, otherwise a kernel interval.
    pub fn wait_deadline(&self, bus: &MemoryBus, timeout_ptr: u64) -> Option<u64> {
        let now = bus.timebase.now();
        if timeout_ptr == 0 {
            return None;
        }
        match bus.read_u64(timeout_ptr).unwrap_or(0) as i64 {
            0 => Some(now),
            interval => super::scheduler::deadline(now, interval),
        }
    }

    /// The guest dispatcher object behind a handle, if the handle is valid and of `kind`.
    pub fn handle_object(&self, handle: u64, kind: super::objects::ObjectKind) -> Result<u64, u64> {
        match self.objects.lookup(handle as u32, self.current_thread) {
            Some(o) if o.kind == kind => Ok(o.guest_ptr as u64),
            Some(_) => Err(super::objects::STATUS_OBJECT_TYPE_MISMATCH),
            None => Err(super::objects::STATUS_INVALID_HANDLE),
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
        // The frontbuffer of the last guest VdSwap is what the GPU scans out (Xenia: XE_SWAP -> IssueSwap).
        if self.last_swap_frontbuffer != 0 && self.last_swap_frontbuffer != 0x7000_0000 {
            return self.last_swap_frontbuffer;
        }
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
