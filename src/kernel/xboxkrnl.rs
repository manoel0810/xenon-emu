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

        krnl!(12, "KeGetCurrentProcess", "Returns pointer to current KPROCESS");
        krnl!(13, "KeGetCurrentProcessType", "Returns process type (1=Title, 2=System)");
        krnl!(40, "ExCreateThread", "Creates a new guest hardware thread");
        krnl!(41, "ExTerminateThread", "Terminates the current thread");
        krnl!(158, "KeInitializeEvent", "Initializes a synchronization event");
        krnl!(160, "KeSetEvent", "Signals an event");
        krnl!(161, "KeWaitForSingleObject", "Waits for an event, mutex, or semaphore");
        krnl!(187, "NtAllocateVirtualMemory", "Allocates virtual address pages");
        krnl!(190, "NtFreeVirtualMemory", "Frees virtual address pages");
        krnl!(202, "NtQueryVirtualMemory", "Queries virtual memory page states");
        krnl!(206, "NtCreateFile", "Opens or creates a file on device filesystem");
        krnl!(208, "NtOpenFile", "Opens an existing file");
        krnl!(220, "RtlInitAnsiString", "Initializes counted ANSI string");
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

        xam!(528, "XamShowSigninUI", "Displays the Xbox 360 Signin blade");
        xam!(532, "XamUserGetSigninState", "Queries current user sign-in status");
        xam!(533, "XamUserGetName", "Retrieves Gamertag string");
        xam!(654, "XamContentCreateEnumerator", "Lists installed content & dashboards");
        xam!(700, "XamShowDeviceSelectorUI", "Displays storage device selection UI");

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
                    // NtAllocateVirtualMemory / NT_STATUS_SUCCESS
                    187 => 0,
                    // DbgPrint
                    405 => 0,
                    _ => 0, // STATUS_SUCCESS
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
                0
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
