//! xam.xex HLE: XamInput*: controller input.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_input(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
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
                let user_index = args.first().copied().unwrap_or(0);
                if user_index == 0 {
                    if let Some(&p_state) = args.get(2) {
                        if p_state != 0 {
                            let _ = bus.write_u32(p_state + 0, self.gamepad_packet_number);
                            let _ = bus.write_u16(p_state + 4, self.gamepad_state.w_buttons);
                            let _ = bus.write_u8(p_state + 6, self.gamepad_state.b_left_trigger);
                            let _ = bus.write_u8(p_state + 7, self.gamepad_state.b_right_trigger);
                            let _ = bus.write_u16(p_state + 8, self.gamepad_state.s_thumb_lx as u16);
                            let _ = bus.write_u16(p_state + 10, self.gamepad_state.s_thumb_ly as u16);
                            let _ = bus.write_u16(p_state + 12, self.gamepad_state.s_thumb_rx as u16);
                            let _ = bus.write_u16(p_state + 14, self.gamepad_state.s_thumb_ry as u16);
                        }
                    }
                    0 // ERROR_SUCCESS
                } else {
                    0x0000_0467 // ERROR_DEVICE_NOT_CONNECTED (1167)
                }
            }

            // 0x0192: XamInputSetState
            0x0192 => 0,

            // 0x0196: XamEnableSystemAppInput
            0x0196 => 0,

            // 0x0198: XamInputGetKeystrokeEx
            0x0198 => 0x0000_0490, // ERROR_EMPTY

            // 0x01B9: XamInputSendStayAliveRequest
            0x01B9 => 0,

            _ => return None,
        })
    }
}
