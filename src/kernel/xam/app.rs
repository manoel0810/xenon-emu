//! xam.xex HLE: XamLoader*/XamApp*/XMsg*/XamTask*/XamAlloc*: title loader, app messaging, tasks and XAM heap.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_app(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x07E2: XamLoaderRegisterLaunchRequestCallback(callback) -> void (stored)
            0x07E2 => {
                self.launch_request_callback = args.first().copied().unwrap_or(0) as u32;
                0
            }

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

            // 0x01A9: XamLoaderTerminateTitle() - does not return: Xenia ends the title (KernelState::TerminateTitle,
            // xam_info.cc); here the frame loop stops emulation.
            0x01A9 => {
                println!("    -> XamLoaderTerminateTitle: title requested exit to the dashboard; stopping emulation");
                self.title_terminated = true;
                0
            }

            // 0x01AA: XamLoaderGetDvdTrayState
            0x01AA => 0, // Tray closed

            // 0x01B1: XamTaskCloseHandle
            0x01B1 => 0,

            // 0x01C7: XamSetPowerMode
            0x01C7 => 0,

            // 0x01CF: XamGetCurrentTitleId
            0x01CF => 0xFFFE_07D1, // Dashboard Title ID

            // 0x01D1: XamAllocHeapFreeSpace
            0x01D1 => 64 * 1024 * 1024,

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

            // 0x01F4: XMsgInProcessCall(app, message, buffer, buffer_length): dispatched synchronously to the app.
            // XLiveBase (0xFC) follows Xenia's XLiveBaseApp::DispatchMessageSync (apps/xlivebase_app.cc); the
            // other apps are not emulated yet and still succeed without doing anything.
            0x01F4 => {
                let (app, message, buffer) = (args.first().copied().unwrap_or(0) as u32, args.get(1).copied().unwrap_or(0) as u32, args.get(2).copied().unwrap_or(0));
                if app == 0xFC { Self::xlivebase_message(bus, message, buffer) } else { 0 }
            }

            // 0x01A3: XamLoaderGetMediaInfo (pMediaInfo*, pFlags*) - no disc inserted
            0x01A3 => {
                for k in 0..=1usize {
                    if let Some(&p) = args.get(k) { if p != 0 && (p & 3) == 0 { let _ = bus.write_u32(p, 0); } }
                }
                0
            }

            // 0x0246: XamAppUnloadStack
            0x0246 => 0,

            // 0x0B45: XamLoaderRebootToServerDash
            0x0B45 => 0,

            _ => return None,
        })
    }
}

impl KernelDispatcher {
    /// XLiveBaseApp::DispatchMessageSync (Xenia apps/xlivebase_app.cc): the console is offline.
    fn xlivebase_message(bus: &mut MemoryBus, message: u32, buffer: u64) -> u64 {
        const X_E_FAIL: u64 = 0x8000_4005;
        const X_ONLINE_E_LOGON_NOT_LOGGED_ON: u64 = 0x8015_1802;
        match message {
            // XLiveBaseLogonGetHR: X_ONLINE_S_LOGON_DISCONNECTED (the dash compares against it, 0x92918A74)
            0x0005_8003 => 0x0015_10F1,
            // XLiveBaseGetLogonId: a bool in the buffer
            0x0005_8004 => {
                if buffer != 0 {
                    let _ = bus.write_u32(buffer, 1);
                }
                0
            }
            // XLiveBaseGetNatType, XLiveBaseOnlineGetServiceInfo: only answered online
            0x0005_8006 | 0x0005_8007 => X_ONLINE_E_LOGON_NOT_LOGGED_ON,
            // XPresenceInitialize, XLiveBaseUnk58046
            0x0005_8037 | 0x0005_8046 => 0,
            // XLiveBaseUnk5008C/50094 (blades dashboards), CXLiveFriends::Enumerate,
            // XMessageGameInviteGetAcceptedInfo and unknown messages
            _ => {
                if crate::diag::config().log_syscalls {
                    println!("    -> XLiveBase message 0x{:08X}: X_E_FAIL", message);
                }
                X_E_FAIL
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn terminate_title_stops_the_title_and_video_flags_match_the_mode() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x01C9, &[], &mut bus), 3, "VdQueryVideoFlags: 1280x720 16:9");
        assert!(!k.title_terminated);
        k.dispatch("xam.xex", 0x01A9, &[], &mut bus);
        assert!(k.title_terminated);
    }

    #[test]
    fn xlivebase_messages_follow_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        assert_eq!(k.dispatch("xam.xex", 0x01F4, &[0xFC, 0x5_8003, 0, 0], &mut bus), 0x0015_10F1);
        assert_eq!(k.dispatch("xam.xex", 0x01F4, &[0xFC, 0x5_8004, 0x4100_0000, 4], &mut bus), 0);
        assert_eq!(bus.read_u32(0x4100_0000).unwrap(), 1);
        assert_eq!(k.dispatch("xam.xex", 0x01F4, &[0xFC, 0x5_8006, 0, 0], &mut bus), 0x8015_1802);
        assert_eq!(k.dispatch("xam.xex", 0x01F4, &[0xFC, 0x5_8037, 0, 0], &mut bus), 0);
        assert_eq!(k.dispatch("xam.xex", 0x01F4, &[0xFC, 0x5_8020, 0, 0], &mut bus), 0x8000_4005);
    }
}
