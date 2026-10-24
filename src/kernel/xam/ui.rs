//! xam.xex HLE: XamShow*/XamUi*: system UI.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_ui(&mut self, ordinal: u16, _args: &[u64], _bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x01CC: XamPartyGetUserList
            0x01CC => 0,

            // 0x01D2: XamSetDashContextEx
            0x01D2 => 0,

            // 0x02BC..0x031D, 0x0492, 0x04A9: System UI Blading and Dialog Services
            0x02BC | 0x02BE | 0x02BF | 0x02D1 | 0x02E3 | 0x02E4 | 0x02F0 | 0x02F8 | 0x02FC | 0x030A | 0x031D | 0x0492 | 0x04A9 => 0,

            _ => return None,
        })
    }
}
