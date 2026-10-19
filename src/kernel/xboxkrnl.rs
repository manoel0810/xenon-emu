use std::collections::HashMap;

pub struct KernelFunction {
    pub name: &'static str,
    pub description: &'static str,
}

pub struct KernelDispatcher {
    pub xboxkrnl_exports: HashMap<u16, KernelFunction>,
    pub xam_exports: HashMap<u16, KernelFunction>,
}

impl KernelDispatcher {
    pub fn new() -> Self {
        let mut xboxkrnl = HashMap::new();
        let mut xam = HashMap::new();

        // Common XboxKrnl Exports
        macro_rules! krnl {
            ($ord:expr, $name:expr, $desc:expr) => {
                xboxkrnl.insert(
                    $ord,
                    KernelFunction {
                        name: $name,
                        description: $desc,
                    },
                );
            };
        }

        krnl!(3, "KeAcquireSpinLockAtDpcLevel", "Acquires spinlock at DPC level");
        krnl!(12, "KeGetCurrentProcess", "Returns pointer to current KPROCESS");
        krnl!(13, "KeGetCurrentProcessType", "Returns process type (1=Title, 2=System)");
        krnl!(15, "KeGetCurrentThread", "Returns pointer to current KTHREAD");
        krnl!(25, "KeQueryPerformanceCounter", "Retrieves high-resolution timer value");
        krnl!(36, "KeInitializeCriticalSection", "Initializes synchronization critical section");
        krnl!(40, "ExCreateThread", "Creates a new guest hardware thread");
        krnl!(41, "ExTerminateThread", "Terminates the current thread");
        krnl!(51, "KeLeaveCriticalSection", "Releases critical section");
        krnl!(52, "KeEnterCriticalSection", "Acquires critical section");
        krnl!(53, "KeInitializeCriticalSectionAndSpinCount", "Initializes spin count critical section");
        krnl!(55, "KeSetEvent", "Signals an event");
        krnl!(56, "KeResetEvent", "Resets an event");
        krnl!(57, "KePulseEvent", "Pulses an event");
        krnl!(60, "KeWaitForSingleObject", "Waits for an event, mutex, or semaphore");
        krnl!(62, "KeWaitForMultipleObjects", "Waits for multiple synchronization objects");
        krnl!(151, "RtlZeroMemory", "Fills memory block with zeros");
        krnl!(153, "RtlFillMemory", "Fills memory with specified byte value");
        krnl!(158, "KeInitializeEvent", "Initializes a synchronization event");
        krnl!(160, "KeSetEvent", "Signals an event");
        krnl!(161, "KeWaitForSingleObject", "Waits for single object");
        krnl!(183, "NtClose", "Closes a kernel object handle");
        krnl!(186, "NtAllocateVirtualMemory", "Allocates virtual address pages");
        krnl!(187, "NtAllocateVirtualMemory", "Allocates virtual address pages");
        krnl!(189, "NtFreeVirtualMemory", "Frees virtual address pages");
        krnl!(190, "NtFreeVirtualMemory", "Frees virtual address pages");
        krnl!(196, "NtQueryVirtualMemory", "Queries virtual memory page states");
        krnl!(202, "NtQueryVirtualMemory", "Queries virtual memory page states");
        krnl!(204, "NtCreateFile", "Opens or creates a file on device filesystem");
        krnl!(206, "NtCreateFile", "Opens or creates a file on device filesystem");
        krnl!(208, "NtOpenFile", "Opens an existing file");
        krnl!(211, "NtDeviceIoControlFile", "Sends control code to device driver");
        krnl!(213, "NtQueryInformationFile", "Queries file metadata and attributes");
        krnl!(220, "RtlInitAnsiString", "Initializes counted ANSI string");
        krnl!(238, "NtQueryInformationFile", "Queries file metadata and attributes");
        krnl!(240, "NtReadFile", "Reads data from a file handle");
        krnl!(242, "NtWriteFile", "Writes data to a file handle");
        krnl!(244, "NtReadFile", "Reads data from a file handle");
        krnl!(248, "RtlMultiByteToUnicodeN", "Converts ANSI string to Unicode");
        krnl!(251, "RtlUnicodeStringToAnsiString", "Converts Unicode string to ANSI");
        krnl!(252, "RtlUnicodeStringToAnsiString", "Converts Unicode string to ANSI");
        krnl!(270, "KeQuerySystemTime", "Retrieves system time in 100ns units");
        krnl!(293, "RtlEnterCriticalSection", "Acquires critical section");
        krnl!(294, "RtlLeaveCriticalSection", "Releases critical section");
        krnl!(299, "XexCheckExecutablePrivilege", "Verifies process privileges");
        krnl!(304, "RtlLeaveCriticalSection", "Releases critical section ownership");
        krnl!(309, "RtlNtStatusToDosError", "Converts NTSTATUS to Win32 error code");
        krnl!(310, "RtlUnicodeToMultiByteSize", "Computes multi-byte size of unicode string");
        krnl!(346, "XapiInitProcess", "Initializes XAPI / CRT runtime environment");
        krnl!(352, "XexUnloadImage", "Unloads an executable module");
        krnl!(402, "DbgBreakPoint", "Triggers debug breakpoint trap");
        krnl!(405, "DbgPrint", "Prints debug string to console");
        krnl!(601, "XexCheckExecutablePrivilege", "Verifies process privileges");
        krnl!(602, "XexGetModuleHandle", "Retrieves handle to loaded module");
        krnl!(603, "XexGetProcedureAddress", "Resolves exported symbol address");

        // Common XAM Exports (Dashboard & User subsystem)
        macro_rules! xam {
            ($ord:expr, $name:expr, $desc:expr) => {
                xam.insert(
                    $ord,
                    KernelFunction {
                        name: $name,
                        description: $desc,
                    },
                );
            };
        }

        xam!(95, "XamAppRegister", "Registers dashboard application with XAM manager");
        xam!(107, "XamAppLookup", "Looks up application registration status");
        xam!(125, "XamAppAttach", "Attaches execution thread to XAM application context");
        xam!(240, "XamGetExecutionId", "Gets module execution ID");
        xam!(253, "XamNetUpnpSearchCreate", "Creates UPnP network search listener");
        xam!(298, "XamLoaderGetLaunchData", "Retrieves system boot/launch data");
        xam!(300, "XamLoaderGetLaunchData", "Retrieves system boot/launch data");
        xam!(400, "XamNotifyCreateListener", "Registers system notification listener");
        xam!(401, "XamNotifyCloseListener", "Closes system notification listener");
        xam!(402, "XamNotifyGetNext", "Retrieves pending notification event");
        xam!(450, "XamContentGetDeviceVolume", "Gets storage root volume mount point");
        xam!(528, "XamShowSigninUI", "Displays the Xbox 360 Signin blade");
        xam!(532, "XamUserGetSigninState", "Queries current user sign-in status");
        xam!(533, "XamUserGetName", "Retrieves Gamertag string");
        xam!(539, "XamUserGetGamerScore", "Queries user Gamerscore");
        xam!(601, "XuiRegisterClass", "Registers XUI visual component class");
        xam!(602, "XuiUnregisterClass", "Unregisters XUI visual component class");
        xam!(603, "XuiVisualSetPosition", "Positions visual element on canvas");
        xam!(604, "XuiElementGetId", "Retrieves XUI element unique identifier");
        xam!(605, "XuiElementGetChildById", "Finds child visual element by ID");
        xam!(606, "XuiElementAddChild", "Attaches child element to scene tree");
        xam!(613, "XuiCreateObject", "Instantiates XUI control object");
        xam!(615, "XuiSceneCreate", "Creates root visual scene hierarchy");
        xam!(617, "XuiRenderInit", "Initializes XUI hardware rendering device");
        xam!(618, "XuiRenderBegin", "Begins graphics frame pass for XUI");
        xam!(630, "XuiVisualGetPosition", "Gets visual element 2D coordinates");
        xam!(650, "XuiRenderEnd", "Ends graphics frame pass for XUI");
        xam!(651, "XuiRenderPresent", "Presents rendered frame buffer to display");
        xam!(654, "XamContentCreateEnumerator", "Lists installed content & dashboards");
        xam!(700, "XamShowDeviceSelectorUI", "Displays storage device selection UI");
        xam!(702, "XamShowKeyboardUI", "Displays virtual keyboard on screen");

        Self {
            xboxkrnl_exports: xboxkrnl,
            xam_exports: xam,
        }
    }

    pub fn resolve_xboxkrnl(&self, ordinal: u16) -> Option<&KernelFunction> {
        self.xboxkrnl_exports.get(&ordinal)
    }

    pub fn resolve_xam(&self, ordinal: u16) -> Option<&KernelFunction> {
        self.xam_exports.get(&ordinal)
    }

    /// Dispatches a kernel syscall call by ordinal and returns mock result status
    pub fn dispatch(&self, library: &str, ordinal: u16, args: &[u64]) -> u64 {
        if library.eq_ignore_ascii_case("xboxkrnl.exe") {
            if let Some(func) = self.resolve_xboxkrnl(ordinal) {
                println!(
                    "  [HLE KERNEL] Call xboxkrnl.exe!{} (#{}) with args: {:016X?}",
                    func.name, ordinal, args
                );
                match ordinal {
                    // KeGetCurrentProcessType -> return 2 (System process for Dashboard)
                    13 => 2,
                    // KeGetCurrentThread -> pointer to current thread object
                    15 => 0x8001_0000,
                    // KeQueryPerformanceCounter
                    25 => 1_000_000,
                    // XexCheckExecutablePrivilege -> 1 (true, privileges granted)
                    299 | 601 => 1,
                    // RtlNtStatusToDosError -> 0 (ERROR_SUCCESS)
                    309 => 0,
                    // Default to STATUS_SUCCESS (0)
                    _ => 0,
                }
            } else {
                println!("  [HLE KERNEL] Unimplemented xboxkrnl.exe ordinal #{}", ordinal);
                0
            }
        } else if library.eq_ignore_ascii_case("xam.xex") {
            if let Some(func) = self.resolve_xam(ordinal) {
                println!(
                    "  [HLE XAM] Call xam.xex!{} (#{}) with args: {:016X?}",
                    func.name, ordinal, args
                );
                match ordinal {
                    // XamUserGetSigninState -> 1 (User signed in locally)
                    532 => 1,
                    // XamNotifyCreateListener -> valid handle
                    400 => 0x0000_2000,
                    // Default to STATUS_SUCCESS (0)
                    _ => 0,
                }
            } else {
                println!("  [HLE XAM] Unimplemented xam.xex ordinal #{}", ordinal);
                0
            }
        } else {
            println!("  [HLE] Unknown library: {} ordinal #{}", library, ordinal);
            0
        }
    }
}
