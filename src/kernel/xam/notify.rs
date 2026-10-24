//! xam.xex HLE: XamNotify*/XNotify*: system notifications.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_notify(&mut self, ordinal: u16, _args: &[u64], _bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x028A: XNotifyCreateListener
            0x028A => {
                let h = self.next_handle;
                self.next_handle += 4;
                h as u64
            }

            // 0x028C | 0x028D | 0x0290 | 0x0292 | 0x02A0 | 0x02A1: Notifications and Dialogs
            0x028C | 0x028D | 0x0290 | 0x0292 | 0x02A0 | 0x02A1 => 0,

            _ => return None,
        })
    }
}
