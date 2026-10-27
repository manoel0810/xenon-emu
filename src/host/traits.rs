//! Abstract traits defining the boundary between the emulator and the host system (Phase 5.1).
//!
//! The guest Xbox 360 kernel and XAM call exports which translate into these operations.
//! Two backends implement them: `HostOffline` (safe, deterministic default) and `HostReal` (OS-backed).

use std::io::SeekFrom;

/// Error type returned by host operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostErr {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    ConnectionRefused,
    Timeout,
    InvalidInput,
    Other(String),
}

impl std::fmt::Display for HostErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "Not found"),
            Self::PermissionDenied => write!(f, "Permission denied"),
            Self::AlreadyExists => write!(f, "Already exists"),
            Self::ConnectionRefused => write!(f, "Connection refused"),
            Self::Timeout => write!(f, "Timed out"),
            Self::InvalidInput => write!(f, "Invalid input"),
            Self::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for HostErr {}

// -------------------------------------------------------------------------------------------------
// Network (HostNet)
// -------------------------------------------------------------------------------------------------

/// Ethernet and network link status reported to the guest (XNetGetEthernetLinkStatus).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    Disconnected,
    Connected {
        ip: [u8; 4],
        gateway: [u8; 4],
        subnet: [u8; 4],
        dns: [u8; 4],
    },
}

pub trait HostNet: Send + Sync {
    /// Returns the current ethernet cable link status.
    fn link_status(&self) -> LinkStatus;

    /// Creates a socket (af, type, protocol).
    fn socket(&mut self, af: i32, ty: i32, proto: i32) -> Result<u32, HostErr>;

    /// Connects a socket to an IPv4 address and port.
    fn connect(&mut self, sock: u32, addr: [u8; 4], port: u16) -> Result<(), HostErr>;

    /// Binds a socket to an address and port.
    fn bind(&mut self, sock: u32, addr: [u8; 4], port: u16) -> Result<(), HostErr>;

    /// Sends raw bytes over a socket.
    fn send(&mut self, sock: u32, buf: &[u8]) -> Result<usize, HostErr>;

    /// Receives raw bytes from a socket.
    fn recv(&mut self, sock: u32, buf: &mut [u8]) -> Result<usize, HostErr>;

    /// Closes a socket.
    fn close_socket(&mut self, sock: u32) -> Result<(), HostErr>;

    /// Resolves a hostname to a list of IPv4 addresses.
    fn dns_resolve(&mut self, name: &str) -> Result<Vec<[u8; 4]>, HostErr>;
}

// -------------------------------------------------------------------------------------------------
// Storage (HostStorage, HostFile)
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenMode {
    Read,
    Write,
    ReadWrite,
    Create,
    CreateNew,
}

pub trait HostFile: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, HostErr>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, HostErr>;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, HostErr>;
    fn size(&self) -> u64;
}

pub trait HostStorage: Send + Sync {
    /// Opens or creates a file at `path`.
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<Box<dyn HostFile>, HostErr>;

    /// Checks if a file or directory exists at `path`.
    fn exists(&self, path: &str) -> bool;

    /// Creates a directory at `path`.
    fn create_dir(&mut self, path: &str) -> Result<(), HostErr>;

    /// Removes a file at `path`.
    fn remove_file(&mut self, path: &str) -> Result<(), HostErr>;

    /// Lists directory entries at `path`.
    fn list_dir(&self, path: &str) -> Result<Vec<String>, HostErr>;
}

// -------------------------------------------------------------------------------------------------
// Time (HostTime)
// -------------------------------------------------------------------------------------------------

pub trait HostTime: Send + Sync {
    /// Returns the current time as a Windows FILETIME (100-nanosecond intervals since Jan 1 1601 UTC).
    fn now_filetime(&self) -> u64;

    /// Returns high-resolution performance counter ticks.
    fn query_performance_counter(&self) -> u64;

    /// Returns high-resolution performance counter frequency in Hz (typically 50,000,000 on Xbox 360).
    fn query_performance_frequency(&self) -> u64;
}

// -------------------------------------------------------------------------------------------------
// Input (HostInput, PadState)
// -------------------------------------------------------------------------------------------------

/// State of an Xbox 360 controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PadState {
    pub w_buttons: u16,
    pub b_left_trigger: u8,
    pub b_right_trigger: u8,
    pub s_thumb_lx: i16,
    pub s_thumb_ly: i16,
    pub s_thumb_rx: i16,
    pub s_thumb_ry: i16,
    pub connected: bool,
}

pub trait HostInput: Send + Sync {
    /// Polls gamepad state for a given user slot (0..3).
    fn poll_pad(&mut self, slot: u32) -> PadState;

    /// Sets vibration motors for a slot.
    fn set_vibration(&mut self, slot: u32, left_motor: u16, right_motor: u16);

    /// Injects or updates the virtual gamepad state (e.g. from host keyboard or window input) for a given slot (0..3).
    fn set_virtual_pad(&mut self, slot: u32, pad: PadState) {
        let _ = (slot, pad);
    }
}

// -------------------------------------------------------------------------------------------------
// Audio (HostAudio)
// -------------------------------------------------------------------------------------------------

pub trait HostAudio: Send + Sync {
    /// Submits 16-bit signed PCM audio samples to the audio device.
    fn submit_pcm(&mut self, samples: &[i16]);

    /// Retrieves and clears buffered PCM samples (useful for inspection, tests or forwarding to an audio device).
    fn drain_pcm(&mut self) -> Vec<i16> {
        Vec::new()
    }
}

// -------------------------------------------------------------------------------------------------
// Profile & User (HostProfile, UserInfo)
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub slot: u32,
    pub xuid: u64,
    pub gamertag: String,
    pub signin_state: u32, // 0: NotSignedIn, 1: SignedInLocally, 2: SignedInToLive
}

pub trait HostProfile: Send + Sync {
    /// Returns all currently signed-in users.
    fn signed_in_users(&self) -> Vec<UserInfo>;

    /// Returns information for a specific user slot (0..3).
    fn get_user(&self, slot: u32) -> Option<UserInfo>;
}

// -------------------------------------------------------------------------------------------------
// Master Host Trait
// -------------------------------------------------------------------------------------------------

/// Master trait bundling all host subsystems.
pub trait Host: Send + Sync {
    fn net(&mut self) -> &mut dyn HostNet;
    fn storage(&mut self) -> &mut dyn HostStorage;
    fn time(&self) -> &dyn HostTime;
    fn input(&mut self) -> &mut dyn HostInput;
    fn audio(&mut self) -> &mut dyn HostAudio;
    fn profile(&self) -> &dyn HostProfile;
}
