/// Xbox 360 USB Gamepad & XInput Controller Integration
/// Supports real USB Xbox 360 / Xbox One gamepads as well as keyboard mapping.

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct XInputGamepad {
    pub w_buttons: u16,
    pub b_left_trigger: u8,
    pub b_right_trigger: u8,
    pub s_thumb_lx: i16,
    pub s_thumb_ly: i16,
    pub s_thumb_rx: i16,
    pub s_thumb_ry: i16,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct XInputState {
    pub dw_packet_number: u32,
    pub gamepad: XInputGamepad,
}

pub const XINPUT_GAMEPAD_DPAD_UP: u16 = 0x0001;
pub const XINPUT_GAMEPAD_DPAD_DOWN: u16 = 0x0002;
pub const XINPUT_GAMEPAD_DPAD_LEFT: u16 = 0x0004;
pub const XINPUT_GAMEPAD_DPAD_RIGHT: u16 = 0x0008;
pub const XINPUT_GAMEPAD_START: u16 = 0x0010;
pub const XINPUT_GAMEPAD_BACK: u16 = 0x0020;
pub const XINPUT_GAMEPAD_LEFT_THUMB: u16 = 0x0040;
pub const XINPUT_GAMEPAD_RIGHT_THUMB: u16 = 0x0080;
pub const XINPUT_GAMEPAD_LEFT_SHOULDER: u16 = 0x0100;
pub const XINPUT_GAMEPAD_RIGHT_SHOULDER: u16 = 0x0200;
pub const XINPUT_GAMEPAD_A: u16 = 0x1000;
pub const XINPUT_GAMEPAD_B: u16 = 0x2000;
pub const XINPUT_GAMEPAD_X: u16 = 0x4000;
pub const XINPUT_GAMEPAD_Y: u16 = 0x8000;

type XInputGetStateFn = unsafe extern "system" fn(u32, *mut XInputState) -> u32;

pub struct GamepadManager {
    get_state_fn: Option<XInputGetStateFn>,
    pub controller_connected: bool,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryA(lpLibFileName: *const u8) -> *mut std::ffi::c_void;
    fn GetProcAddress(hModule: *mut std::ffi::c_void, lpProcName: *const u8) -> *mut std::ffi::c_void;
}

impl GamepadManager {
    pub fn new() -> Self {
        let mut get_state_fn = None;
        let dll_names: &[&[u8]] = &[b"xinput1_4.dll\0", b"xinput1_3.dll\0", b"xinput9_1_0.dll\0"];

        for &dll in dll_names {
            unsafe {
                let handle = LoadLibraryA(dll.as_ptr());
                if !handle.is_null() {
                    let proc_name = b"XInputGetState\0";
                    let proc = GetProcAddress(handle, proc_name.as_ptr());
                    if !proc.is_null() {
                        get_state_fn = Some(std::mem::transmute::<*mut std::ffi::c_void, XInputGetStateFn>(proc));
                        break;
                    }
                }
            }
        }

        let mut mgr = Self {
            get_state_fn,
            controller_connected: false,
        };

        // Probe user 0
        if let Some(state) = mgr.poll(0) {
            mgr.controller_connected = true;
            println!("  [USB INPUT] Xbox 360 / XInput Gamepad detected on Port 0! (Buttons: 0x{:04X})", state.gamepad.w_buttons);
        } else {
            println!("  [USB INPUT] No physical gamepad detected on Port 0; Keyboard emulation active.");
        }

        mgr
    }

    /// Polls physical Xbox controller on user_index (0..3)
    pub fn poll(&mut self, user_index: u32) -> Option<XInputState> {
        if let Some(func) = self.get_state_fn {
            let mut state = XInputState::default();
            let res = unsafe { func(user_index, &mut state) };
            if res == 0 {
                self.controller_connected = true;
                return Some(state);
            }
        }
        self.controller_connected = false;
        None
    }
}
