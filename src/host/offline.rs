//! Safe, deterministic offline host implementation (Phase 5.1).
//!
//! Provides zero external network I/O, deterministic simulated time, mock in-memory storage,
//! neutral gamepad state, and a default local profile ("Xbox Player").

use std::collections::HashMap;
use std::io::SeekFrom;
use std::sync::atomic::{AtomicU64, Ordering};
use super::traits::{
    Host, HostAudio, HostErr, HostFile, HostInput, HostNet, HostProfile, HostStorage, HostTime,
    LinkStatus, OpenMode, PadState, UserInfo,
};

// -------------------------------------------------------------------------------------------------
// Offline Network
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct OfflineNet;

impl HostNet for OfflineNet {
    fn link_status(&self) -> LinkStatus {
        LinkStatus::Disconnected
    }

    fn socket(&mut self, _af: i32, _ty: i32, _proto: i32) -> Result<u32, HostErr> {
        Err(HostErr::ConnectionRefused)
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

    fn dns_resolve(&mut self, _name: &str) -> Result<Vec<[u8; 4]>, HostErr> {
        Err(HostErr::NotFound)
    }
}

// -------------------------------------------------------------------------------------------------
// Offline Storage
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OfflineFile {
    pub path: String,
    pub data: Vec<u8>,
    pub cursor: usize,
}

impl HostFile for OfflineFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, HostErr> {
        if self.cursor >= self.data.len() {
            return Ok(0);
        }
        let available = &self.data[self.cursor..];
        let bytes_to_copy = std::cmp::min(buf.len(), available.len());
        buf[..bytes_to_copy].copy_from_slice(&available[..bytes_to_copy]);
        self.cursor += bytes_to_copy;
        Ok(bytes_to_copy)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, HostErr> {
        let end_pos = self.cursor + buf.len();
        if end_pos > self.data.len() {
            self.data.resize(end_pos, 0);
        }
        self.data[self.cursor..end_pos].copy_from_slice(buf);
        self.cursor = end_pos;
        Ok(buf.len())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, HostErr> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.data.len() as i64 + offset,
            SeekFrom::Current(offset) => self.cursor as i64 + offset,
        };
        if new_pos < 0 {
            return Err(HostErr::InvalidInput);
        }
        self.cursor = new_pos as usize;
        Ok(self.cursor as u64)
    }

    fn size(&self) -> u64 {
        self.data.len() as u64
    }
}

#[derive(Debug, Default)]
pub struct OfflineStorage {
    pub files: HashMap<String, Vec<u8>>,
    pub directories: Vec<String>,
}

impl HostStorage for OfflineStorage {
    fn open(&mut self, path: &str, mode: OpenMode) -> Result<Box<dyn HostFile>, HostErr> {
        match mode {
            OpenMode::Read => {
                let data = self.files.get(path).ok_or(HostErr::NotFound)?.clone();
                Ok(Box::new(OfflineFile {
                    path: path.to_string(),
                    data,
                    cursor: 0,
                }))
            }
            OpenMode::Write | OpenMode::ReadWrite => {
                let data = self.files.entry(path.to_string()).or_default().clone();
                Ok(Box::new(OfflineFile {
                    path: path.to_string(),
                    data,
                    cursor: 0,
                }))
            }
            OpenMode::Create => {
                self.files.insert(path.to_string(), Vec::new());
                Ok(Box::new(OfflineFile {
                    path: path.to_string(),
                    data: Vec::new(),
                    cursor: 0,
                }))
            }
            OpenMode::CreateNew => {
                if self.files.contains_key(path) {
                    return Err(HostErr::AlreadyExists);
                }
                self.files.insert(path.to_string(), Vec::new());
                Ok(Box::new(OfflineFile {
                    path: path.to_string(),
                    data: Vec::new(),
                    cursor: 0,
                }))
            }
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.directories.iter().any(|d| d == path)
    }

    fn create_dir(&mut self, path: &str) -> Result<(), HostErr> {
        if !self.directories.iter().any(|d| d == path) {
            self.directories.push(path.to_string());
        }
        Ok(())
    }

    fn remove_file(&mut self, path: &str) -> Result<(), HostErr> {
        self.files.remove(path).map(|_| ()).ok_or(HostErr::NotFound)
    }

    fn list_dir(&self, path: &str) -> Result<Vec<String>, HostErr> {
        let prefix = if path.ends_with('/') || path.ends_with('\\') || path.is_empty() {
            path.to_string()
        } else {
            format!("{}/", path)
        };

        let mut results = Vec::new();
        for key in self.files.keys() {
            if key.starts_with(&prefix) {
                let sub = &key[prefix.len()..];
                let entry = sub.split(['/', '\\']).next().unwrap_or(sub);
                if !results.iter().any(|r: &String| r == entry) {
                    results.push(entry.to_string());
                }
            }
        }
        Ok(results)
    }
}

// -------------------------------------------------------------------------------------------------
// Offline Time (Deterministic)
// -------------------------------------------------------------------------------------------------

/// Base FILETIME: Approx Nov 2012 (130_000_000_000_000_000)
pub const OFFLINE_BASE_FILETIME: u64 = 130_000_000_000_000_000;
pub const XBOX360_PERF_FREQ: u64 = 50_000_000;

pub struct OfflineTime {
    counter: AtomicU64,
}

impl Default for OfflineTime {
    fn default() -> Self {
        Self {
            counter: AtomicU64::new(1_000_000),
        }
    }
}

impl OfflineTime {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance_ticks(&self, ticks: u64) {
        self.counter.fetch_add(ticks, Ordering::Relaxed);
    }
}

impl HostTime for OfflineTime {
    fn now_filetime(&self) -> u64 {
        let ticks = self.counter.fetch_add(50_000, Ordering::Relaxed);
        // 50,000 ticks at 50MHz is 1ms = 10,000 * 100ns units
        OFFLINE_BASE_FILETIME + (ticks / 5)
    }

    fn query_performance_counter(&self) -> u64 {
        self.counter.fetch_add(50_000, Ordering::Relaxed)
    }

    fn query_performance_frequency(&self) -> u64 {
        XBOX360_PERF_FREQ
    }
}

// -------------------------------------------------------------------------------------------------
// Offline Input
// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
pub struct OfflineInput {
    pub pads: [PadState; 4],
}

impl Default for OfflineInput {
    fn default() -> Self {
        let mut pads = [PadState::default(); 4];
        // Port 0 is connected with neutral stick & no buttons
        pads[0].connected = true;
        Self { pads }
    }
}

impl HostInput for OfflineInput {
    fn poll_pad(&mut self, slot: u32) -> PadState {
        self.pads.get(slot as usize).copied().unwrap_or_default()
    }

    fn set_vibration(&mut self, _slot: u32, _left_motor: u16, _right_motor: u16) {}
}

// -------------------------------------------------------------------------------------------------
// Offline Audio
// -------------------------------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct OfflineAudio {
    pub total_samples_submitted: usize,
}

impl HostAudio for OfflineAudio {
    fn submit_pcm(&mut self, samples: &[i16]) {
        self.total_samples_submitted += samples.len();
    }
}

// -------------------------------------------------------------------------------------------------
// Offline Profile
// -------------------------------------------------------------------------------------------------

pub struct OfflineProfile {
    user: UserInfo,
}

impl Default for OfflineProfile {
    fn default() -> Self {
        Self {
            user: UserInfo {
                slot: 0,
                xuid: 0x0009_0000_0000_0001,
                gamertag: "Xbox Player".to_string(),
                signin_state: 1, // SignedInLocally
            },
        }
    }
}

impl HostProfile for OfflineProfile {
    fn signed_in_users(&self) -> Vec<UserInfo> {
        vec![self.user.clone()]
    }

    fn get_user(&self, slot: u32) -> Option<UserInfo> {
        if slot == 0 {
            Some(self.user.clone())
        } else {
            None
        }
    }
}

// -------------------------------------------------------------------------------------------------
// Master HostOffline
// -------------------------------------------------------------------------------------------------

pub struct HostOffline {
    pub net: OfflineNet,
    pub storage: OfflineStorage,
    pub time: OfflineTime,
    pub input: OfflineInput,
    pub audio: OfflineAudio,
    pub profile: OfflineProfile,
}

impl Default for HostOffline {
    fn default() -> Self {
        Self {
            net: OfflineNet::default(),
            storage: OfflineStorage::default(),
            time: OfflineTime::default(),
            input: OfflineInput::default(),
            audio: OfflineAudio::default(),
            profile: OfflineProfile::default(),
        }
    }
}

impl HostOffline {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Host for HostOffline {
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
