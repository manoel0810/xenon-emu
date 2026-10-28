// Derived from Xenia (BSD-3-Clause), Copyright (c) 2022 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. xam_msg.cc (xeXMsgStartIORequestEx), app_manager.cc, apps/xmp_app.{h,cc} and
// apps/xlivebase_app.cc (DispatchMessageSync), Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de.
//
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

            // 0x01F7: XMsgStartIORequest(app, message, overlapped, buffer, buffer_length)
            // 0x01FC: XMsgStartIORequestEx(app, message, overlapped, buffer, buffer_length, task_args)
            // xeXMsgStartIORequestEx (xam_msg.cc): the app handles the message at once; with an overlapped it is
            // completed with that result and the call returns X_ERROR_IO_PENDING. Only the emulated apps (XMP 0xFA,
            // XLiveBase 0xFC) answer; the others stay unhandled. Not ported: SetLastError.
            0x01F7 | 0x01FC => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0) & 0xFFFF_FFFF;
                let (app, message, overlapped, buffer) = (arg(0) as u32, arg(1) as u32, arg(2), arg(3));
                let result = match app {
                    0xFA => self.xmp_message(bus, message, buffer),
                    0xFC => Self::xlivebase_message(bus, message, buffer),
                    _ => return None,
                };
                if overlapped != 0 {
                    self.complete_overlapped_immediate(bus, overlapped, result as u32);
                    997 // X_ERROR_IO_PENDING
                } else {
                    result
                }
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
    /// XmpApp::DispatchMessageSync (Xenia apps/xmp_app.cc), for the messages the dash sends at startup. No media
    /// player is emulated; the other messages fail as Xenia's unimplemented ones do (X_E_FAIL).
    fn xmp_message(&mut self, bus: &mut MemoryBus, message: u32, buffer: u64) -> u64 {
        const X_E_FAIL: u64 = 0x8000_4005;
        const X_E_INVALIDARG: u64 = 0x8007_0057;
        // XMP_CLIENT
        const DASH: u32 = 0;
        const HUD: u32 = 1;
        const GAME: u32 = 2;
        let field = |bus: &MemoryBus, i: u64| bus.read_u32(buffer + 4 * i).unwrap_or(0);
        match message {
            // XMPCreateTitlePlaylist {xmp_client, storage_ptr, storage_size, songs_ptr, song_count, playlist_name_ptr,
            // flags, song_handles_ptr, playlist_handle_ptr}: new song and playlist handles; the playlist handle goes
            // to the storage, whose address goes to playlist_handle_ptr (kXNotificationXmpTitlePlayListContentChanged).
            // The songs are not kept: no media player is emulated.
            0x0007_000D => {
                let (storage_ptr, songs_ptr, song_count) = (field(bus, 1), field(bus, 3), field(bus, 4));
                let (song_handles_ptr, playlist_handle_ptr) = (field(bus, 7) as u64, field(bus, 8) as u64);
                if playlist_handle_ptr != 0 {
                    let _ = bus.write_u32(playlist_handle_ptr, storage_ptr);
                }
                self.xmp_handles.0 += 1;
                if songs_ptr != 0 {
                    for i in 0..song_count as u64 {
                        self.xmp_handles.1 += 1;
                        if song_handles_ptr != 0 {
                            let _ = bus.write_u32(song_handles_ptr + 4 * i, self.xmp_handles.1);
                        }
                    }
                }
                if storage_ptr != 0 {
                    let _ = bus.write_u32(storage_ptr as u64, self.xmp_handles.0);
                }
                self.notifications.broadcast(bus, crate::kernel::notify::XMP_TITLE_PLAYLIST_CONTENT_CHANGED, 0);
                0
            }
            // XMPGetTitlePlaylistBufferSize {xmp_client, song_count, size_ptr}: room for the songs, not used here
            0x0007_002E => {
                let (client, song_count, size_ptr) = (field(bus, 0), field(bus, 1), field(bus, 2) as u64);
                if client == HUD || size_ptr == 0 || song_count == 0 {
                    return X_E_INVALIDARG;
                }
                let size = if client == DASH || client == GAME { song_count.wrapping_mul(0x3E8).wrapping_add(0x88) } else { 0 };
                let _ = bus.write_u32(size_ptr, size);
                0
            }
            // XMPDashInit {xmp_client, buffer_ptr, buffer_length, unk1, unk2, storage_ptr}: announces the dash's
            // music player (kXNotificationXmpDashInitChanged, 1)
            0x0007_002F => {
                let (client, buffer_ptr, storage_ptr) = (field(bus, 0), field(bus, 1), field(bus, 5));
                if client != DASH || storage_ptr == 0 || buffer_ptr == 0 {
                    return X_E_INVALIDARG;
                }
                self.notifications.broadcast(bus, crate::kernel::notify::XMP_DASH_INIT_CHANGED, 1);
                0
            }
            // XMPSetMediaSourceWorkspace {xmp_client, workspace_type, storage_ptr, storage_length}
            0x0007_0044 => {
                let (client, workspace_type, storage_ptr) = (field(bus, 0), field(bus, 1), field(bus, 2));
                if client > GAME || (storage_ptr == 0 && workspace_type != 3) {
                    return X_E_INVALIDARG;
                }
                0
            }
            _ => {
                if crate::diag::config().log_syscalls {
                    println!("    -> XMP message 0x{:08X}: X_E_FAIL", message);
                }
                X_E_FAIL
            }
        }
    }

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

    #[test]
    fn xmp_start_io_requests_follow_xenia() {
        use crate::kernel::notify::{Listener, NOTIFY_XMP, XMP_DASH_INIT_CHANGED};
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        k.notifications.register(&mut bus, 1, Listener { mask: NOTIFY_XMP, max_version: 0, event: 0x4100_0F00, queue: Default::default() }, true, false);
        let put = |bus: &mut MemoryBus, words: &[u32]| {
            for (i, &w) in words.iter().enumerate() {
                bus.write_u32(0x4100_0000 + 4 * i as u64, w).unwrap();
            }
        };
        let start = |k: &mut KernelDispatcher, bus: &mut MemoryBus, message: u64, overlapped: u64, length: u64| {
            k.dispatch("xam.xex", 0x01FC, &[0xFA, message, overlapped, 0x4100_0000, length, 0], bus)
        };
        // XMPGetTitlePlaylistBufferSize as the dash asks at startup (dash client, 9000 songs, as on the console)
        put(&mut bus, &[0, 9000, 0x4100_0100]);
        assert_eq!(start(&mut k, &mut bus, 0x7_002E, 0, 0xC), 0);
        assert_eq!(bus.read_u32(0x4100_0100).unwrap(), 9000 * 0x3E8 + 0x88);
        put(&mut bus, &[1, 9000, 0x4100_0100]);
        assert_eq!(start(&mut k, &mut bus, 0x7_002E, 0, 0xC), 0x8007_0057, "HUD client");
        // XMPCreateTitlePlaylist: two songs get handles 1 and 2, the playlist handle 1 goes to the storage, the
        // storage address to playlist_handle_ptr
        put(&mut bus, &[0, 0x4100_0500, 0x100, 0x4100_0600, 2, 0, 0, 0x4100_0700, 0x4100_0800]);
        assert_eq!(start(&mut k, &mut bus, 0x7_000D, 0, 0x24), 0);
        let read = |bus: &MemoryBus, a: u64| bus.read_u32(a).unwrap();
        assert_eq!((read(&bus, 0x4100_0700), read(&bus, 0x4100_0704), read(&bus, 0x4100_0500), read(&bus, 0x4100_0800)), (1, 2, 1, 0x4100_0500));
        assert_eq!(k.notifications.listeners.get_mut(&1).unwrap().dequeue(&mut bus, 0), Some((crate::kernel::notify::XMP_TITLE_PLAYLIST_CONTENT_CHANGED, 0)));
        // XMPDashInit: the dash client with buffers announces the dash's player
        put(&mut bus, &[0, 0x4100_0200, 0x10, 0, 0, 0x4100_0300]);
        assert_eq!(start(&mut k, &mut bus, 0x7_002F, 0, 0x18), 0);
        assert_eq!(k.notifications.listeners.get_mut(&1).unwrap().dequeue(&mut bus, 0), Some((XMP_DASH_INIT_CHANGED, 1)));
        put(&mut bus, &[2, 0x4100_0200, 0x10, 0, 0, 0x4100_0300]);
        assert_eq!(start(&mut k, &mut bus, 0x7_002F, 0, 0x18), 0x8007_0057);
        // XMPSetMediaSourceWorkspace: a storage, or workspace type 3 without one
        put(&mut bus, &[0, 1, 0x4100_0300, 0x100]);
        assert_eq!(start(&mut k, &mut bus, 0x7_0044, 0, 0x10), 0);
        put(&mut bus, &[0, 1, 0, 0]);
        assert_eq!(start(&mut k, &mut bus, 0x7_0044, 0, 0x10), 0x8007_0057);
        put(&mut bus, &[0, 3, 0, 0]);
        assert_eq!(start(&mut k, &mut bus, 0x7_0044, 0, 0x10), 0);
        // not ported: X_E_FAIL; with an overlapped the result goes there and the call is pending
        assert_eq!(start(&mut k, &mut bus, 0x7_0099, 0, 0), 0x8000_4005);
        assert_eq!(start(&mut k, &mut bus, 0x7_0044, 0x4100_0400, 0x10), 997);
        assert_eq!(bus.read_u32(0x4100_0400).unwrap(), 0);
        // apps that are not emulated stay unhandled
        assert_eq!(k.xam_app(0x01FC, &[0xFE, 0x2_5005, 0, 0x4100_0000, 4, 0], &mut bus), None);
    }
}
