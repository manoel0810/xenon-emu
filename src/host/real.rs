//! OS-backed real host implementation (Phase 5.1).
//!
//! Provides real filesystem access via `std::fs`, real system time via `SystemTime` and `Instant`,
//! physical gamepad access via XInput on Windows, and real networking capabilities.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use super::traits::{
    Host, HostAudio, HostErr, HostFile, HostInput, HostNet, HostProfile, HostStorage, HostTime,
    LinkStatus, OpenMode, PadState, UserInfo,
};

// -------------------------------------------------------------------------------------------------
// Real Network
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct RealNet;

impl HostNet for RealNet {
    fn link_status(&self) -> LinkStatus {
        // Report connected to loopback/local network by default
        LinkStatus::Connected {
            ip: [127, 0, 0, 1],
            gateway: [127, 0, 0, 1],
            subnet: [255, 255, 255, 0],
            dns: [127, 0, 0, 1],
        }
    }

    fn socket(&mut self, _af: i32, _ty: i32, _proto: i32) -> Result<u32, HostErr> {
        // Will be expanded in Phase 5.3
        Err(HostErr::Other("Socket creation not yet attached to real OS socket".to_string()))
    }

    fn connect(&mut self, _sock: u32, _addr: [u8; 4], _port: u16) -> Result<(), HostErr> {
        Err(HostErr::ConnectionRefused)
    }

    fn bind(&mut self, _sock: u32, _addr: [u8; 4], _port: u16) -> Result<(), HostErr> {
        Err(HostErr::ConnectionRefused)
    }

    fn send(&mut self, _sock: u32, _buf: &[u8]) -> Result<usize, HostErr> {
        Err(HostErr::ConnectionRefused)
    }

    fn recv(&mut self, _sock: u32, _buf: &mut [u8]) -> Result<usize, HostErr> {
        Err(HostErr::ConnectionRefused)
    }

    fn close_socket(&mut self, _sock: u32) -> Result<(), HostErr> {
        Ok(())
    }

    fn dns_resolve(&mut self, name: &str) -> Result<Vec<[u8; 4]>, HostErr> {
        use std::net::ToSocketAddrs;
        let formatted = format!("{}:80", name);
        if let Ok(iter) = formatted.to_socket_addrs() {
            let mut ips = Vec::new();
            for addr in iter {
                if let std::net::SocketAddr::V4(v4) = addr {
                    ips.push(v4.ip().octets());
                }
            }
            if !ips.is_empty() {
                return Ok(ips);
            }
        }
        Err(HostErr::NotFound)
    }
}

// -------------------------------------------------------------------------------------------------
// Real Storage & File
// -------------------------------------------------------------------------------------------------

pub struct RealFile {
    file: File,
}

impl HostFile for RealFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, HostErr> {
        self.file.read(buf).map_err(|e| HostErr::Other(e.to_string()))
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, HostErr> {
        self.file.write(buf).map_err(|e| HostErr::Other(e.to_string()))
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, HostErr> {
        self.file.seek(pos).map_err(|e| HostErr::Other(e.to_string()))
    }

    fn size(&self) -> u64 {
        self.file.metadata().map(|m| m.len()).unwrap_or(0)
    }
}

#[derive(Debug)]
pub struct RealStorage {
    root_dir: PathBuf,
}

impl Default for RealStorage {
    fn default() -> Self {
        Self {
            root_dir: PathBuf::from("."),
        }
    }
}

impl RealStorage {
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root_dir: root.as_ref().to_path_buf(),
        }
    }

    fn resolve_path(&self, guest_path: &str) -> PathBuf {
        let clean = guest_path.trim_start_matches(['/', '\\']);
        self.root_dir.join(clean)
    }
}

impl HostStorage for RealStorage {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<Box<dyn HostFile>, HostErr> {
        let target = self.resolve_path(path);
        let mut opts = OpenOptions::new();
        match mode {
            OpenMode::Read => {
                opts.read(true);
            }
            OpenMode::Write => {
                opts.write(true).create(true);
            }
            OpenMode::ReadWrite => {
                opts.read(true).write(true).create(true);
            }
            OpenMode::Create => {
                opts.read(true).write(true).create(true).truncate(true);
            }
            OpenMode::CreateNew => {
                opts.read(true).write(true).create_new(true);
            }
        }

        let file = opts.open(&target).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => HostErr::NotFound,
            std::io::ErrorKind::PermissionDenied => HostErr::PermissionDenied,
            std::io::ErrorKind::AlreadyExists => HostErr::AlreadyExists,
            _ => HostErr::Other(e.to_string()),
        })?;

        Ok(Box::new(RealFile { file }))
    }

    fn exists(&self, path: &str) -> bool {
        self.resolve_path(path).exists()
    }

    fn create_dir(&mut self, path: &str) -> Result<(), HostErr> {
        fs::create_dir_all(self.resolve_path(path)).map_err(|e| HostErr::Other(e.to_string()))
    }

    fn remove_file(&mut self, path: &str) -> Result<(), HostErr> {
        fs::remove_file(self.resolve_path(path)).map_err(|e| HostErr::Other(e.to_string()))
    }

    fn list_dir(&self, path: &str) -> Result<Vec<String>, HostErr> {
        let entries = fs::read_dir(self.resolve_path(path)).map_err(|e| HostErr::Other(e.to_string()))?;
        let mut list = Vec::new();
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                list.push(name);
            }
        }
        Ok(list)
    }
}

// -------------------------------------------------------------------------------------------------
// Real Time (System Clock & High-Res Instant)
// -------------------------------------------------------------------------------------------------

/// Windows FILETIME epoch offset: 11644473600 seconds from 1601-01-01 to 1970-01-01.
pub const FILETIME_UNIX_DIFF_SECS: u64 = 11_644_473_600;

pub struct RealTime {
    start_instant: Instant,
}

impl Default for RealTime {
    fn default() -> Self {
        Self {
            start_instant: Instant::now(),
        }
    }
}

impl RealTime {
    pub fn new() -> Self {
        Self::default()
    }
}

impl HostTime for RealTime {
    fn now_filetime(&self) -> u64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(dur) => {
                let unix_100ns = dur.as_secs() * 10_000_000 + dur.subsec_nanos() as u64 / 100;
                unix_100ns + (FILETIME_UNIX_DIFF_SECS * 10_000_000)
            }
            Err(_) => 130_000_000_000_000_000,
        }
    }

    fn query_performance_counter(&self) -> u64 {
        let elapsed = self.start_instant.elapsed();
        // Scale to 50 MHz (50 ticks per microsecond)
        let secs_part = elapsed.as_secs().saturating_mul(50_000_000);
        let nanos_part = (elapsed.subsec_nanos() as u64).saturating_mul(50) / 1_000;
        secs_part.saturating_add(nanos_part)
    }

    fn query_performance_frequency(&self) -> u64 {
        50_000_000
    }
}

// -------------------------------------------------------------------------------------------------
// Real Input (Physical XInput Gamepad on Windows)
// -------------------------------------------------------------------------------------------------

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct RawXInputGamepad {
    w_buttons: u16,
    b_left_trigger: u8,
    b_right_trigger: u8,
    s_thumb_lx: i16,
    s_thumb_ly: i16,
    s_thumb_rx: i16,
    s_thumb_ry: i16,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct RawXInputState {
    dw_packet_number: u32,
    gamepad: RawXInputGamepad,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
struct RawXInputVibration {
    w_left_motor_speed: u16,
    w_right_motor_speed: u16,
}

type XInputGetStateFn = unsafe extern "system" fn(u32, *mut RawXInputState) -> u32;
type XInputSetStateFn = unsafe extern "system" fn(u32, *const RawXInputVibration) -> u32;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryA(lpLibFileName: *const u8) -> *mut std::ffi::c_void;
    fn GetProcAddress(hModule: *mut std::ffi::c_void, lpProcName: *const u8) -> *mut std::ffi::c_void;
}

pub struct RealInput {
    get_state_fn: Option<XInputGetStateFn>,
    set_state_fn: Option<XInputSetStateFn>,
}

impl Default for RealInput {
    fn default() -> Self {
        Self::new()
    }
}

impl RealInput {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            let mut get_state_fn = None;
            let mut set_state_fn = None;
            let dll_names: &[&[u8]] = &[b"xinput1_4.dll\0", b"xinput1_3.dll\0", b"xinput9_1_0.dll\0"];

            for &dll in dll_names {
                unsafe {
                    let handle = LoadLibraryA(dll.as_ptr());
                    if !handle.is_null() {
                        let proc_get = GetProcAddress(handle, b"XInputGetState\0".as_ptr());
                        if !proc_get.is_null() {
                            get_state_fn = Some(std::mem::transmute::<*mut std::ffi::c_void, XInputGetStateFn>(proc_get));
                        }
                        let proc_set = GetProcAddress(handle, b"XInputSetState\0".as_ptr());
                        if !proc_set.is_null() {
                            set_state_fn = Some(std::mem::transmute::<*mut std::ffi::c_void, XInputSetStateFn>(proc_set));
                        }
                        if get_state_fn.is_some() {
                            break;
                        }
                    }
                }
            }

            Self {
                get_state_fn,
                set_state_fn,
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                get_state_fn: None,
                set_state_fn: None,
            }
        }
    }
}

impl HostInput for RealInput {
    fn poll_pad(&mut self, slot: u32) -> PadState {
        if let Some(func) = self.get_state_fn {
            let mut state = RawXInputState::default();
            let res = unsafe { func(slot, &mut state) };
            if res == 0 {
                return PadState {
                    w_buttons: state.gamepad.w_buttons,
                    b_left_trigger: state.gamepad.b_left_trigger,
                    b_right_trigger: state.gamepad.b_right_trigger,
                    s_thumb_lx: state.gamepad.s_thumb_lx,
                    s_thumb_ly: state.gamepad.s_thumb_ly,
                    s_thumb_rx: state.gamepad.s_thumb_rx,
                    s_thumb_ry: state.gamepad.s_thumb_ry,
                    connected: true,
                };
            }
        }
        PadState {
            connected: false,
            ..Default::default()
        }
    }

    fn set_vibration(&mut self, slot: u32, left_motor: u16, right_motor: u16) {
        if let Some(func) = self.set_state_fn {
            let vib = RawXInputVibration {
                w_left_motor_speed: left_motor,
                w_right_motor_speed: right_motor,
            };
            unsafe {
                func(slot, &vib);
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Real Audio
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct RealAudio {
    pub total_samples_written: usize,
    buffer: Vec<i16>,
}

impl HostAudio for RealAudio {
    fn submit_pcm(&mut self, samples: &[i16]) {
        self.total_samples_written += samples.len();
        if self.buffer.len() < 96_000 {
            let take = samples.len().min(96_000 - self.buffer.len());
            self.buffer.extend_from_slice(&samples[..take]);
        }
    }

    fn drain_pcm(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.buffer)
    }
}

// -------------------------------------------------------------------------------------------------
// Real Profile
// -------------------------------------------------------------------------------------------------

pub struct RealProfile {
    gamertag: String,
    xuid: u64,
}

impl Default for RealProfile {
    fn default() -> Self {
        let user_name = std::env::var("USERNAME").unwrap_or_else(|_| "Xbox Player".to_string());
        Self {
            gamertag: user_name,
            xuid: 0x0009_0000_0000_0001,
        }
    }
}

impl HostProfile for RealProfile {
    fn signed_in_users(&self) -> Vec<UserInfo> {
        vec![UserInfo {
            slot: 0,
            xuid: self.xuid,
            gamertag: self.gamertag.clone(),
            signin_state: 1, // SignedInLocally
        }]
    }

    fn get_user(&self, slot: u32) -> Option<UserInfo> {
        if slot == 0 {
            Some(UserInfo {
                slot: 0,
                xuid: self.xuid,
                gamertag: self.gamertag.clone(),
                signin_state: 1,
            })
        } else {
            None
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Master HostReal
// -------------------------------------------------------------------------------------------------

pub struct HostReal {
    pub net: RealNet,
    pub storage: RealStorage,
    pub time: RealTime,
    pub input: RealInput,
    pub audio: RealAudio,
    pub profile: RealProfile,
}

impl Default for HostReal {
    fn default() -> Self {
        Self {
            net: RealNet::default(),
            storage: RealStorage::default(),
            time: RealTime::default(),
            input: RealInput::default(),
            audio: RealAudio::default(),
            profile: RealProfile::default(),
        }
    }
}

impl HostReal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_storage_root<P: AsRef<Path>>(root: P) -> Self {
        let mut host = Self::default();
        host.storage = RealStorage::new(root);
        host
    }
}

impl Host for HostReal {
    fn net(&mut self) -> &mut dyn HostNet {
        &mut self.net
    }

    fn storage(&mut self) -> &mut dyn HostStorage {
        &mut self.storage
    }

    fn time(&self) -> &dyn HostTime {
        &self.time
    }

    fn input(&mut self) -> &mut dyn HostInput {
        &mut self.input
    }

    fn audio(&mut self) -> &mut dyn HostAudio {
        &mut self.audio
    }

    fn profile(&self) -> &dyn HostProfile {
        &self.profile
    }
}
