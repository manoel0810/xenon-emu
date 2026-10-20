use std::collections::HashMap;
use crate::memory::MemoryBus;
use super::exports::{get_xboxkrnl_name, get_xam_name};

pub struct KernelFunction {
    pub name: &'static str,
    pub description: &'static str,
}

pub struct KernelDispatcher {
    pub next_thread_id: u32,
    pub next_handle: u32,
    pub tls_slots: HashMap<u32, u64>,
    pub next_tls_index: u32,
    pub valloc_bump: u64,
    pub xam_app_registered: bool,
    pub xam_app_id: u32,
    pub signin_state: u32,
    pub perf_counter: u64,
}

impl KernelDispatcher {
    pub fn new() -> Self {
        Self {
            next_thread_id: 1,
            next_handle: 0x0000_1000,
            tls_slots: HashMap::new(),
            next_tls_index: 1,
            valloc_bump: 0x7000_0000,
            xam_app_registered: false,
            xam_app_id: 0,
            signin_state: 1,
            perf_counter: 10_000_000,
        }
    }

    pub fn resolve_xboxkrnl(&self, ordinal: u16) -> Option<&'static str> {
        get_xboxkrnl_name(ordinal)
    }

    pub fn resolve_xam(&self, ordinal: u16) -> Option<&'static str> {
        get_xam_name(ordinal)
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

        println!(
            "  [HLE SYSCALL] {}!{} (#0x{:04X} / {}) with args: {:016X?}",
            library, func_name, ordinal, ordinal, args
        );

        if is_krnl {
            self.dispatch_xboxkrnl(ordinal, func_name, args, bus)
        } else {
            self.dispatch_xam(ordinal, func_name, args, bus)
        }
    }

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
                println!("    -> ExCreateThread: Created thread TID=0x{:X}, Handle=0x{:X}", tid, h_thread);
                0 // STATUS_SUCCESS
            }

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

            // 0x0019: ExTerminateThread
            0x0019 => {
                println!("    -> ExTerminateThread: Thread exited cleanly.");
                0
            }

            // 0x0024: HalOpenCloseODDTray
            0x0024 => 0,

            // 0x0029: HalSendSMCMessage
            0x0029 => 0,

            // 0x004D: KeAcquireSpinLockAtRaisedIrql
            0x004D => 0,

            // 0x0052: KeBugCheck | 0x0053: KeBugCheckEx
            0x0052 | 0x0053 => {
                println!("    [!] GUEST KeBugCheck called with code 0x{:X}", args.first().unwrap_or(&0));
                0
            }

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
                        for off in (0..16).step_by(4) {
                            let _ = bus.write_u32(p_event + off, 0);
                        }
                        let initial = args.get(2).copied().unwrap_or(0);
                        let _ = bus.write_u8(p_event + 4, initial as u8);
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

            // 0x0081: KeQueryBasePriorityThread
            0x0081 => 8, // Normal thread priority

            // 0x0085: KeRaiseIrqlToDpcLevel
            0x0085 => 2, // Previous IRQL (DISPATCH_LEVEL)

            // 0x0088: KeReleaseSemaphore
            0x0088 => 1,

            // 0x0084: KeAcquireSpinLockAtRaisedIrql | 0x0089: KeReleaseSpinLockFromRaisedIrql
            0x0084 | 0x0089 => 0,

            // 0x008F: KeResetEvent
            0x008F => {
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        let _ = bus.write_u8(p_event + 4, 0);
                    }
                }
                0
            }

            // 0x0092: KeResumeThread
            0x0092 => 1, // Previous suspend count

            // 0x0097: KeSetAffinityThread
            0x0097 => 1,

            // 0x0099: KeSetBasePriorityThread
            0x0099 => 0,

            // 0x009D: KeSetEvent
            0x009D => {
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        let _ = bus.write_u8(p_event + 4, 1);
                    }
                }
                0
            }

            // 0x00B0: KeWaitForSingleObject
            0x00B0 => 0, // STATUS_SUCCESS / WAIT_OBJECT_0

            // 0x00B1: KfAcquireSpinLock
            0x00B1 => 0,

            // 0x00B3: KfLowerIrql
            0x00B3 => 0,

            // 0x00B4: KfReleaseSpinLock
            0x00B4 => 0,

            // 0x00BA: MmAllocatePhysicalMemoryEx
            // (type, size, protect, minAddr, maxAddr, alignment)
            0x00BA => {
                let size = args.get(1).copied().unwrap_or(0x1000).max(0x1000);
                let aligned = (size + 0xFFF) & !0xFFF;
                let addr = self.valloc_bump;
                self.valloc_bump += aligned;
                println!("    -> MmAllocatePhysicalMemoryEx: Allocated 0x{:X} bytes at 0x{:08X}", aligned, addr);
                addr
            }

            // 0x00BD: MmFreePhysicalMemory
            0x00BD => 0,

            // 0x00BE: MmGetPhysicalAddress
            0x00BE => args.first().copied().unwrap_or(0) & 0x1FFF_FFFF,

            // 0x00C4: MmQueryAddressProtect
            0x00C4 => 0x04, // PAGE_READWRITE

            // 0x00C5: MmQueryAllocationSize
            0x00C5 => 0x1000,

            // 0x00CC: NtAllocateVirtualMemory
            // (pBase, zeroBits, pSize, allocType, protect)
            0x00CC => {
                let p_base = args.first().copied().unwrap_or(0);
                let p_size = args.get(2).copied().unwrap_or(0);
                let mut size = 0x1000u64;
                if p_size != 0 {
                    if let Ok(sz) = bus.read_u32(p_size) {
                        if sz > 0 {
                            size = sz as u64;
                        }
                    }
                }
                let aligned = (size + 0xFFF) & !0xFFF;
                let addr = self.valloc_bump;
                self.valloc_bump += aligned;

                if p_base != 0 {
                    let _ = bus.write_u32(p_base, addr as u32);
                }
                if p_size != 0 {
                    let _ = bus.write_u32(p_size, aligned as u32);
                }
                println!("    -> NtAllocateVirtualMemory: Allocated 0x{:X} bytes at 0x{:08X}", aligned, addr);
                0 // STATUS_SUCCESS
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

            // 0x00DC: NtFreeVirtualMemory
            0x00DC => 0,

            // 0x00E8: NtQueryInformationFile
            0x00E8 => 0,

            // 0x00EE: NtQueryVirtualMemory
            0x00EE => 0,

            // 0x00EF: NtQueryVolumeInformationFile
            0x00EF => 0,

            // 0x00F4: NtRemoveIoCompletion
            0x00F4 => 0x0000_0102, // STATUS_TIMEOUT

            // 0x00F6: NtSetEvent
            0x00F6 => 0,

            // 0x00F8: NtSetIoCompletion
            0x00F8 => 0,

            // 0x00FC: NtSuspendThread
            0x00FC => 0,

            // 0x00FF: NtWriteFile
            0x00FF => 0,

            // 0x0101: NtYieldExecution
            0x0101 => 0,

            // 0x0103: ObCreateSymbolicLink | 0x0104: ObDeleteSymbolicLink
            0x0103 | 0x0104 => 0,

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

            // 0x0126: RtlFillMemoryUlong (dst, pattern, count)
            0x0126 => {
                if let (Some(&dst), Some(&pat), Some(&cnt)) = (args.first(), args.get(1), args.get(2)) {
                    for i in 0..cnt {
                        let _ = bus.write_u32(dst + i * 4, pat as u32);
                    }
                }
                0
            }

            // 0x012B: RtlImageXexHeaderField
            0x012B => 0,

            // 0x012E: RtlInitializeCriticalSection
            0x012E => {
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

            // 0x0199: XexLoadImage | 0x01A1: XexUnloadImage | 0x01A2: XexUnloadImageAndExitThread
            0x0199 | 0x01A1 | 0x01A2 => 0,

            // 0x01A5: __C_specific_handler
            0x01A5 => 0,

            // 0x01B1..0x01B9: Video Display (Vd*) Handlers
            0x01B1 | 0x01B3 | 0x01B4 | 0x01B5 | 0x01B6 | 0x01B7 | 0x01B8 | 0x01B9 => 0,

            // 0x01BA: VdGetCurrentDisplayInformation
            0x01BA => 0,

            // 0x01BD: VdGetSystemCommandBuffer
            0x01BD => 0x1000_0000,

            // 0x01BE: VdGlobalDevice
            0x01BE => 0x1001_0000,

            // 0x01C0: VdGpuClockInMHz
            0x01C0 => 500, // 500 MHz ATI Xenos

            // 0x01C1: VdHSIOCalibrationLock
            0x01C1 => 0,

            // 0x01C2: VdInitializeEngines | 0x01C3: VdInitializeRingBuffer | 0x01C5: VdInitializeScalerCommandBuffer
            0x01C2 | 0x01C3 | 0x01C5 => 0,

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
            0x01CF | 0x01D0 | 0x01D3 | 0x01D4 | 0x01D5 | 0x01D9 | 0x01DC => 0,

            // 0x01DF: KiApcNormalRoutineNop
            0x01DF => 0,

            // 0x01E7: XInputdWriteState | 0x01E8: XInputdNotify
            0x01E7 | 0x01E8 => 0,

            // 0x01F3..0x0200: XAudio Handlers
            0x01F3 | 0x01F4 | 0x01F5 | 0x01F7 | 0x01F8 | 0x01FA | 0x01FB | 0x01FC | 0x0200 => 0,

            // 0x021C: XUsbcamSetConfig | 0x0221: XUsbcamGetView
            0x021C | 0x0221 => 0,

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

            // 0x025B: VdSwap
            0x025B => 0,

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
            | 0x000F | 0x0010 | 0x0011 | 0x0012 | 0x0013 | 0x0014 | 0x0016 | 0x0017 | 0x0018 => 0,

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
                if let Some(&p_state) = args.get(2) {
                    if p_state != 0 {
                        let _ = bus.write_u32(p_state + 0, 1); // dwPacketNumber
                        let _ = bus.write_u16(p_state + 4, 0); // wButtons
                        let _ = bus.write_u8(p_state + 6, 0);  // bLeftTrigger
                        let _ = bus.write_u8(p_state + 7, 0);  // bRightTrigger
                        let _ = bus.write_u16(p_state + 8, 0);  // sThumbLX
                        let _ = bus.write_u16(p_state + 10, 0); // sThumbLY
                        let _ = bus.write_u16(p_state + 12, 0); // sThumbRX
                        let _ = bus.write_u16(p_state + 14, 0); // sThumbRY
                    }
                }
                0 // ERROR_SUCCESS
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

            // 0x01C5: XamGetDefaultSystemImage
            0x01C5 => 0,

            // 0x01C7: XamSetPowerMode
            0x01C7 => 0,

            // 0x01CF: XamGetCurrentTitleId
            0x01CF => 0xFFFE_07D1, // Dashboard Title ID

            // 0x01D2: XamSetDashContextEx
            0x01D2 => 0,

            // 0x01F5: XamGetExecutionId / XMsgCompleteIORequest
            // Writes genuine Dashboard XEX_EXECUTION_ID (24 bytes)
            0x01F5 => {
                if let Some(&p_exec) = args.first() {
                    if p_exec != 0 {
                        let _ = bus.write_u32(p_exec + 0, 0x0000_0000); // media_id
                        let _ = bus.write_u32(p_exec + 4, 0x2011_B400); // version (2.0.4532.0)
                        let _ = bus.write_u32(p_exec + 8, 0x2011_B400); // base_version
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

            // 0x021B: XamUserGetMembershipTier
            0x021B => 6, // Gold Membership

            // 0x0230: XamProfileCreate | 0x0234: XamProfileGetCreationStatus
            0x0230 | 0x0234 => 0,

            // 0x0250: XamEnumerate
            0x0250 => 0x0000_0103, // STATUS_NO_MORE_FILES

            // 0x0259: XamContentCreateEx
            0x0259 => 0,

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

            // 0x0290: XNotifyQueueUI
            0x0290 => 0,

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

            // 0x02BC..0x031D: System UI Blading and Dialog Services
            0x02BC | 0x02BE | 0x02BF | 0x02D1 | 0x02E3 | 0x02E4 | 0x02F0 | 0x02F8 | 0x02FC | 0x030A | 0x031D => 0,

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

            // 0x09C4: XamBackgroundDownloadSetMode
            0x09C4 => 0,

            _ => {
                println!("    [HLE XAM] Unhandled xam.xex ordinal 0x{:04X} ({})", ordinal, ordinal);
                0
            }
        }
    }
}
