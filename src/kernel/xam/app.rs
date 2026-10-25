//! xam.xex HLE: XamLoader*/XamApp*/XMsg*/XamTask*/XamAlloc*: title loader, app messaging, tasks and XAM heap.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_app(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
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

            // 0x01F4: XMsgInProcessCall
            0x01F4 => 0,

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
}
