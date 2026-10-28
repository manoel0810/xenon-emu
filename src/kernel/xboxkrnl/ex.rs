// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. xboxkrnl_xconfig.cc (xeExGetXConfigSetting, ExSetXConfigSetting,
// ExReadModifyWriteXConfigSettingUlong), Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de.
//
//! xboxkrnl.exe HLE: Ex*: executive (threads, pools, XConfig, read/write locks).

use super::super::dispatcher::{KernelDispatcher, PendingThread};
use super::super::objects::KThreadInit;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_ex(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x000D: ExCreateThread
            // (pHandle, stackSize, pThreadId, apiStartup, startRoutine, startContext, creationFlags)
            0x000D => {
                let tid = self.next_thread_id;
                self.next_thread_id += 1;
                let start_routine = args.get(4).copied().unwrap_or(0);
                let stack_size = args.get(1).copied().unwrap_or(0x10000).max(0x10000) as u32;
                let creation_flags = args.get(6).copied().unwrap_or(0) as u32;
                // the stack itself is placed by the frame loop when the thread starts (app/threads.rs)
                let init = KThreadInit { thread_id: tid, start_address: start_routine as u32, stack_base: 0, stack_limit: 0, creation_flags };
                let (h_thread, kthread) = self.create_thread_object(bus, &init);

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

                let start_context = args.get(5).copied().unwrap_or(0);

                println!("    -> ExCreateThread: Created thread TID=0x{:X}, Handle=0x{:X}, Entry=0x{:08X}, Param=0x{:08X}, Stack=0x{:X}",
                    tid, h_thread, start_routine, start_context, stack_size
                );

                if start_routine != 0 {
                    self.pending_threads.push(PendingThread {
                        tid,
                        handle: h_thread,
                        kthread,
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

            // 0x0010: ExGetXConfigSetting(category, setting, buffer, buffer_size, required_size_ptr)
            // (xeExGetXConfigSetting, xboxkrnl_xconfig.cc): X_STATUS_INVALID_PARAMETER_3 for a size without a
            // buffer, X_STATUS_BUFFER_TOO_SMALL below the setting's size; the required size is written either way
            // (0 on failure, 0 too for a setting that is not in the field table, whose read leaves the buffer as is).
            0x0010 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let (category, setting) = ((arg(0) & 0xFFFF) as u16, (arg(1) & 0xFFFF) as u16);
                let (buffer, buffer_size, required_ptr) = (arg(2) & 0xFFFF_FFFF, arg(3) & 0xFFFF, arg(4) & 0xFFFF_FFFF);
                let size = self.xconfig.setting_size(category, setting);
                let (status, required) = if buffer == 0 && buffer_size != 0 {
                    (0xC000_00F1, 0) // X_STATUS_INVALID_PARAMETER_3
                } else if buffer != 0 && buffer_size < size as u64 {
                    (0xC000_0023, 0) // X_STATUS_BUFFER_TOO_SMALL
                } else {
                    if buffer != 0 {
                        if let Some(bytes) = self.xconfig.read(category, setting) {
                            let _ = bus.load_bytes(buffer, bytes);
                        }
                    }
                    (0, size)
                };
                if required_ptr != 0 {
                    let _ = bus.write_u16(required_ptr, required);
                }
                status
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

            // 0x0015: ExRegisterTitleTerminateNotification(registration, create) -> void. Xenia keeps the routine to
            // call when the title exits; titles never exit here, so nothing is kept.
            0x0015 => 0,

            // 0x0018: ExSetXConfigSetting(category, setting, buffer, size) -> X_STATUS_SUCCESS (xboxkrnl_xconfig.cc):
            // the setting's size is copied from the buffer, whatever the size argument says
            0x0018 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let (category, setting, buffer) = ((arg(0) & 0xFFFF) as u16, (arg(1) & 0xFFFF) as u16, arg(2) & 0xFFFF_FFFF);
                let size = self.xconfig.setting_size(category, setting) as u64;
                if buffer != 0 && size != 0 {
                    let data: Vec<u8> = (0..size).map(|i| bus.read_u8(buffer + i).unwrap_or(0)).collect();
                    self.xconfig.write(category, setting, &data);
                }
                0
            }

            // 0x02BC: ExReadModifyWriteXConfigSettingUlong(category, setting, bit_affected, flag) -> X_STATUS_SUCCESS
            // (xboxkrnl_xconfig.cc): value = (value & bit_affected) | flag
            0x02BC => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let (category, setting) = ((arg(0) & 0xFFFF) as u16, (arg(1) & 0xFFFF) as u16);
                let value = (self.xconfig.read_u32(category, setting) & arg(2) as u32) | arg(3) as u32;
                self.xconfig.write(category, setting, &value.to_be_bytes());
                0
            }

            // 0x0019: ExTerminateThread
            0x0019 => 0,

            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn xconfig_settings_follow_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        let get = |k: &mut KernelDispatcher, bus: &mut MemoryBus, category: u64, setting: u64, size: u64| {
            bus.write_u32(0x4100_0000, 0xAAAA_AAAA).unwrap();
            bus.write_u16(0x4100_0100, 0xFFFF).unwrap();
            let status = k.dispatch("xboxkrnl.exe", 0x0010, &[category, setting, 0x4100_0000, size, 0x4100_0100], bus);
            (status, bus.read_u32(0x4100_0000).unwrap(), bus.read_u16(0x4100_0100).unwrap())
        };
        // XCONFIG_USER_RETAIL_FLAGS: DashboardInitialized (the dash's 0x921827D8 tests bit 0x40)
        assert_eq!(get(&mut k, &mut bus, 3, 0x0C, 4), (0, 0x40, 4));
        // too small a buffer, a size without a buffer
        assert_eq!(get(&mut k, &mut bus, 3, 0x0C, 2), (0xC000_0023, 0xAAAA_AAAA, 0));
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0010, &[3, 0x0C, 0, 4, 0x4100_0100], &mut bus), 0xC000_00F1);
        // a setting outside the field table: success, buffer untouched, no size
        assert_eq!(get(&mut k, &mut bus, 7, 0x09, 4), (0, 0xAAAA_AAAA, 0));
        // XCONFIG_USER_COUNTRY is one byte; the language is English
        assert_eq!(get(&mut k, &mut bus, 3, 0x0E, 4), (0, 0x67AA_AAAA, 1));
        assert_eq!(get(&mut k, &mut bus, 3, 0x09, 4).1, 1);
        // ExSetXConfigSetting stores the value; ExReadModifyWriteXConfigSettingUlong keeps bit_affected, adds flag
        bus.write_u32(0x4100_0200, 0x0001_0000).unwrap();
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0018, &[3, 0x0A, 0x4100_0200, 4], &mut bus), 0);
        assert_eq!(get(&mut k, &mut bus, 3, 0x0A, 4).1, 0x0001_0000);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x02BC, &[3, 0x0C, 0x40, 0x8], &mut bus), 0);
        assert_eq!(get(&mut k, &mut bus, 3, 0x0C, 4).1, 0x48);
    }
}
