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
                let user_index = args.first().copied().unwrap_or(0) as u32;
                if user_index >= 4 {
                    return Some(0x0000_0467); // ERROR_DEVICE_NOT_CONNECTED (1167)
                }
                let pad = self.host.input().poll_pad(user_index);
                if !pad.connected {
                    return Some(0x0000_0467); // ERROR_DEVICE_NOT_CONNECTED
                }
                if let Some(&p_state) = args.get(2) {
                    if p_state != 0 {
                        self.gamepad_packet_number = self.gamepad_packet_number.wrapping_add(1);
                        let _ = bus.write_u32(p_state + 0, self.gamepad_packet_number);
                        let _ = bus.write_u16(p_state + 4, pad.w_buttons);
                        let _ = bus.write_u8(p_state + 6, pad.b_left_trigger);
                        let _ = bus.write_u8(p_state + 7, pad.b_right_trigger);
                        let _ = bus.write_u16(p_state + 8, pad.s_thumb_lx as u16);
                        let _ = bus.write_u16(p_state + 10, pad.s_thumb_ly as u16);
                        let _ = bus.write_u16(p_state + 12, pad.s_thumb_rx as u16);
                        let _ = bus.write_u16(p_state + 14, pad.s_thumb_ry as u16);
                    }
                }
                0 // ERROR_SUCCESS
            }

            // 0x0192: XamInputSetState (userIndex, flags, pVibration)
            0x0192 => {
                let user_index = args.first().copied().unwrap_or(0) as u32;
                if user_index < 4 {
                    if let Some(&p_vib) = args.get(2) {
                        if p_vib != 0 {
                            let left = bus.read_u16(p_vib).unwrap_or(0);
                            let right = bus.read_u16(p_vib + 2).unwrap_or(0);
                            self.host.input().set_vibration(user_index, left, right);
                        }
                    }
                    0 // ERROR_SUCCESS
                } else {
                    0x0000_0467 // ERROR_DEVICE_NOT_CONNECTED
                }
            }

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

#[cfg(test)]
mod tests {
    use crate::kernel::dispatcher::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn test_xam_input_get_state() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();

        let state_ptr = 0x4000_0000;
        // Port 0 is connected in offline default
        assert_eq!(k.dispatch("xam.xex", 0x0191, &[0, 0, state_ptr], &mut bus), 0);
        let packet_num = bus.read_u32(state_ptr).unwrap();
        assert!(packet_num > 0);

        // Port 1 is disconnected
        assert_eq!(k.dispatch("xam.xex", 0x0191, &[1, 0, state_ptr], &mut bus), 0x0000_0467);
    }

    #[test]
    fn test_xam_input_set_state() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();

        let vib_ptr = 0x4000_0100;
        bus.write_u16(vib_ptr, 1000).unwrap();
        bus.write_u16(vib_ptr + 2, 2000).unwrap();
        assert_eq!(k.dispatch("xam.xex", 0x0192, &[0, 0, vib_ptr], &mut bus), 0);
    }
}
