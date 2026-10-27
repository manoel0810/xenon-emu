//! xam.xex HLE: XamShow*/XamUi*: system UI.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_ui(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x01CC: XamSetDashContext(value) -> 0 (Xenia xam_info.cc: stores it and broadcasts
            // kXNotificationSystemDashContextChanged when it changes)
            0x01CC => {
                let value = args.first().copied().unwrap_or(0) as u32;
                if value != self.dash_context {
                    self.dash_context = value;
                    self.notifications.broadcast(bus, crate::kernel::notify::SYSTEM_DASH_CONTEXT_CHANGED, 0);
                }
                0
            }

            // 0x01D2: XamSetDashContextEx
            0x01D2 => 0,

            // system UI exports 0x02BC..0x030A, 0x0492
            0x02BC | 0x02BE | 0x02BF | 0x02D1 | 0x02E3 | 0x02E4 | 0x02F0 | 0x02F8 | 0x02FC | 0x030A | 0x0492 => 0,

            _ => return None,
        })
    }
}
