//! Unified Virtual File System (VFS) for Xbox 360 guest paths (Phase 5.2).
//!
//! Handles NT device paths (`\Device\Harddisk0\Partition1\`, `\Device\Cdrom0\`, `\Device\Flash\`),
//! DOS drive letters and symlinks (`hdd0:`, `dvd:`, `game:`, `systemroot:`),
//! memory devices, host storage mounting, and STFS container mounting.

use std::collections::HashMap;
use std::io::SeekFrom;
use crate::host::traits::OpenMode;
use crate::loader::stfs::StfsPackage;

/// VFS error status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VfsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotSupported,
    InvalidPath,
    Other(String),
}

impl VfsError {
    /// Maps VfsError to Windows / Xbox 360 NTSTATUS code.
    pub fn to_ntstatus(&self) -> u32 {
        match self {
            Self::NotFound => 0xC000_0034,       // STATUS_OBJECT_NAME_NOT_FOUND
            Self::PermissionDenied => 0xC000_0022, // STATUS_ACCESS_DENIED
            Self::AlreadyExists => 0xC000_0035,   // STATUS_OBJECT_NAME_COLLISION
            Self::InvalidPath => 0xC000_0033,     // STATUS_OBJECT_NAME_INVALID
            Self::NotSupported => 0xC000_0002,   // STATUS_NOT_IMPLEMENTED
            Self::Other(_) => 0xC000_0001,       // STATUS_UNSUCCESSFUL
        }
    }
}

// -------------------------------------------------------------------------------------------------
// VfsFile trait & MemoryFile implementation
// -------------------------------------------------------------------------------------------------

pub trait VfsFile: Send + Sync {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, VfsError>;
    fn write(&mut self, buf: &[u8]) -> Result<usize, VfsError>;
    fn seek(&mut self, pos: SeekFrom) -> Result<u64, VfsError>;
    fn size(&self) -> u64;
    fn position(&self) -> u64;
}

#[derive(Debug, Clone)]
pub struct MemoryFile {
    pub data: Vec<u8>,
    pub cursor: usize,
}

impl MemoryFile {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data, cursor: 0 }
    }
}

impl VfsFile for MemoryFile {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, VfsError> {
        if self.cursor >= self.data.len() {
            return Ok(0);
        }
        let available = &self.data[self.cursor..];
        let bytes_to_copy = std::cmp::min(buf.len(), available.len());
        buf[..bytes_to_copy].copy_from_slice(&available[..bytes_to_copy]);
        self.cursor += bytes_to_copy;
        Ok(bytes_to_copy)
    }

    fn write(&mut self, buf: &[u8]) -> Result<usize, VfsError> {
        let end_pos = self.cursor + buf.len();
        if end_pos > self.data.len() {
            self.data.resize(end_pos, 0);
        }
        self.data[self.cursor..end_pos].copy_from_slice(buf);
        self.cursor = end_pos;
        Ok(buf.len())
    }

    fn seek(&mut self, pos: SeekFrom) -> Result<u64, VfsError> {
        let new_pos = match pos {
            SeekFrom::Start(offset) => offset as i64,
            SeekFrom::End(offset) => self.data.len() as i64 + offset,
            SeekFrom::Current(offset) => self.cursor as i64 + offset,
        };
        if new_pos < 0 {
            return Err(VfsError::InvalidPath);
        }
        self.cursor = new_pos as usize;
        Ok(self.cursor as u64)
    }

    fn size(&self) -> u64 {
        self.data.len() as u64
    }

    fn position(&self) -> u64 {
        self.cursor as u64
    }
}

// -------------------------------------------------------------------------------------------------
// VfsDevice Trait
// -------------------------------------------------------------------------------------------------

pub trait VfsDevice: Send + Sync {
    fn device_name(&self) -> &str;
    fn open(&mut self, relative_path: &str, mode: OpenMode) -> Result<Box<dyn VfsFile>, VfsError>;
    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError>;
    fn write_file(&mut self, relative_path: &str, data: &[u8]) -> Result<(), VfsError>;
    fn exists(&self, relative_path: &str) -> bool;
    fn list_dir(&self, relative_path: &str) -> Result<Vec<String>, VfsError>;
}

// -------------------------------------------------------------------------------------------------
// MemoryDevice
// -------------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct MemoryDevice {
    name: String,
    files: HashMap<String, Vec<u8>>,
}

impl MemoryDevice {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            files: HashMap::new(),
        }
    }

    fn clean_path(path: &str) -> String {
        path.trim_matches(['/', '\\']).to_lowercase().replace('/', "\\")
    }

    pub fn insert(&mut self, relative_path: &str, data: Vec<u8>) {
        let key = Self::clean_path(relative_path);
        self.files.insert(key, data);
    }
}

impl VfsDevice for MemoryDevice {
    fn device_name(&self) -> &str {
        &self.name
    }

    fn open(&mut self, relative_path: &str, mode: OpenMode) -> Result<Box<dyn VfsFile>, VfsError> {
        let key = Self::clean_path(relative_path);
        match mode {
            OpenMode::Read => {
                let data = self.files.get(&key).ok_or(VfsError::NotFound)?.clone();
                Ok(Box::new(MemoryFile::new(data)))
            }
            OpenMode::Write | OpenMode::ReadWrite => {
                let data = self.files.entry(key).or_default().clone();
                Ok(Box::new(MemoryFile::new(data)))
            }
            OpenMode::Create => {
                self.files.insert(key, Vec::new());
                Ok(Box::new(MemoryFile::new(Vec::new())))
            }
            OpenMode::CreateNew => {
                if self.files.contains_key(&key) {
                    return Err(VfsError::AlreadyExists);
                }
                self.files.insert(key, Vec::new());
                Ok(Box::new(MemoryFile::new(Vec::new())))
            }
        }
    }

    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
        let key = Self::clean_path(relative_path);
        self.files.get(&key).cloned().ok_or(VfsError::NotFound)
    }

    fn write_file(&mut self, relative_path: &str, data: &[u8]) -> Result<(), VfsError> {
        let key = Self::clean_path(relative_path);
        self.files.insert(key, data.to_vec());
        Ok(())
    }

    fn exists(&self, relative_path: &str) -> bool {
        let key = Self::clean_path(relative_path);
        self.files.contains_key(&key)
    }

    fn list_dir(&self, relative_path: &str) -> Result<Vec<String>, VfsError> {
        let prefix = Self::clean_path(relative_path);
        let mut results = Vec::new();
        for k in self.files.keys() {
            if prefix.is_empty() || k.starts_with(&prefix) {
                let rest = if prefix.is_empty() {
                    k.as_str()
                } else {
                    &k[prefix.len()..].trim_start_matches('\\')
                };
                let first_part = rest.split('\\').next().unwrap_or(rest);
                if !first_part.is_empty() && !results.iter().any(|r: &String| r == first_part) {
                    results.push(first_part.to_string());
                }
            }
        }
        Ok(results)
    }
}

// -------------------------------------------------------------------------------------------------
// HostDirectoryDevice
// -------------------------------------------------------------------------------------------------

/// A host directory exposed read-only, with case-insensitive names (the console's FAT/flash file systems are
/// case-insensitive: the dash asks for `XenonJKLatin.xtt`, the flash holds `xenonjklatin.xtt`).
pub struct HostDirectoryDevice {
    name: String,
    root: std::path::PathBuf,
}

impl HostDirectoryDevice {
    pub fn new(name: &str, root: impl Into<std::path::PathBuf>) -> Self {
        Self { name: name.to_string(), root: root.into() }
    }

    /// The host path of `relative_path`, matching each component case-insensitively.
    fn host_path(&self, relative_path: &str) -> Option<std::path::PathBuf> {
        let mut path = self.root.clone();
        for part in relative_path.split(['\\', '/']).filter(|p| !p.is_empty()) {
            let entry = std::fs::read_dir(&path).ok()?.flatten().find(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(part))?;
            path = entry.path();
        }
        Some(path)
    }
}

impl VfsDevice for HostDirectoryDevice {
    fn device_name(&self) -> &str {
        &self.name
    }

    fn open(&mut self, relative_path: &str, mode: OpenMode) -> Result<Box<dyn VfsFile>, VfsError> {
        if mode != OpenMode::Read {
            return Err(VfsError::PermissionDenied);
        }
        Ok(Box::new(MemoryFile::new(self.read_file(relative_path)?)))
    }

    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
        let path = self.host_path(relative_path).filter(|p| p.is_file()).ok_or(VfsError::NotFound)?;
        std::fs::read(path).map_err(|e| VfsError::Other(e.to_string()))
    }

    fn write_file(&mut self, _relative_path: &str, _data: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::PermissionDenied)
    }

    fn exists(&self, relative_path: &str) -> bool {
        self.host_path(relative_path).is_some()
    }

    fn list_dir(&self, relative_path: &str) -> Result<Vec<String>, VfsError> {
        let path = self.host_path(relative_path).filter(|p| p.is_dir()).ok_or(VfsError::NotFound)?;
        let entries = std::fs::read_dir(path).map_err(|e| VfsError::Other(e.to_string()))?;
        Ok(entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
    }
}

// -------------------------------------------------------------------------------------------------
// StfsDevice
// -------------------------------------------------------------------------------------------------

pub struct StfsDevice {
    name: String,
    package: StfsPackage,
    package_bytes: Vec<u8>,
}

impl StfsDevice {
    pub fn new(name: &str, package_bytes: Vec<u8>) -> Result<Self, String> {
        let package = StfsPackage::parse(&package_bytes)?;
        Ok(Self {
            name: name.to_string(),
            package,
            package_bytes,
        })
    }
}

impl VfsDevice for StfsDevice {
    fn device_name(&self) -> &str {
        &self.name
    }

    fn open(&mut self, relative_path: &str, mode: OpenMode) -> Result<Box<dyn VfsFile>, VfsError> {
        if mode != OpenMode::Read {
            return Err(VfsError::NotSupported);
        }
        let data = self.read_file(relative_path)?;
        Ok(Box::new(MemoryFile::new(data)))
    }

    fn read_file(&self, relative_path: &str) -> Result<Vec<u8>, VfsError> {
        let clean = relative_path.trim_matches(['/', '\\']);
        let fname = clean.rsplit(['/', '\\']).next().unwrap_or(clean);
        self.package
            .extract_file(&self.package_bytes, fname)
            .map_err(|_| VfsError::NotFound)
    }

    fn write_file(&mut self, _relative_path: &str, _data: &[u8]) -> Result<(), VfsError> {
        Err(VfsError::NotSupported)
    }

    fn exists(&self, relative_path: &str) -> bool {
        let clean = relative_path.trim_matches(['/', '\\']);
        let fname = clean.rsplit(['/', '\\']).next().unwrap_or(clean);
        self.package.entries.iter().any(|e| e.name.eq_ignore_ascii_case(fname))
    }

    fn list_dir(&self, _relative_path: &str) -> Result<Vec<String>, VfsError> {
        Ok(self.package.entries.iter().map(|e| e.name.clone()).collect())
    }
}

// -------------------------------------------------------------------------------------------------
// VirtualFileSystem
// -------------------------------------------------------------------------------------------------

pub struct VirtualFileSystem {
    devices: Vec<(String, Box<dyn VfsDevice>)>,
    symlinks: HashMap<String, String>,
}

impl Default for VirtualFileSystem {
    fn default() -> Self {
        let mut vfs = Self {
            devices: Vec::new(),
            symlinks: HashMap::new(),
        };

        // Standard Xbox 360 DOS symlinks -> NT device targets
        vfs.add_symlink("hdd0:", "\\device\\harddisk0\\partition1");
        vfs.add_symlink("dvd:", "\\device\\cdrom0");
        vfs.add_symlink("cdrom0:", "\\device\\cdrom0");
        vfs.add_symlink("game:", "\\device\\harddisk0\\partition1\\game");
        vfs.add_symlink("systemroot:", "\\device\\flash");
        // the console's media: drive is the flash (xbdm drivelist/dirlist list the same files)
        vfs.add_symlink("media:", "\\device\\flash");
        vfs.add_symlink("flash:", "\\device\\flash");
        vfs.add_symlink("devkit:", "\\device\\harddisk0\\partition1");
        vfs.add_symlink("d:", "\\device\\cdrom0");

        // Standard mount points
        vfs.mount("\\device\\harddisk0\\partition1", Box::new(MemoryDevice::new("hdd0")));
        vfs.mount("\\device\\cdrom0", Box::new(MemoryDevice::new("dvd")));
        vfs.mount("\\device\\flash", Box::new(MemoryDevice::new("flash")));

        vfs
    }
}

impl VirtualFileSystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mount(&mut self, prefix: &str, device: Box<dyn VfsDevice>) {
        let clean_prefix = prefix.trim_end_matches(['/', '\\']).to_lowercase().replace('/', "\\");
        // Replace existing device if prefix matches
        if let Some(pos) = self.devices.iter().position(|(p, _)| p == &clean_prefix) {
            self.devices[pos] = (clean_prefix, device);
        } else {
            self.devices.push((clean_prefix, device));
            // Keep devices sorted longest-prefix first for correct matching
            self.devices.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        }
    }

    pub fn add_symlink(&mut self, link: &str, target: &str) {
        let clean_link = link.trim_end_matches(['/', '\\']).to_lowercase().replace('/', "\\");
        let clean_target = target.trim_end_matches(['/', '\\']).to_lowercase().replace('/', "\\");
        self.symlinks.insert(clean_link, clean_target);
    }

    /// Normalizes an incoming guest path, resolves symlinks, and strips NT namespaces (`\??\`, `\\?\`).
    pub fn normalize_path(&self, raw_path: &str) -> String {
        let mut p = raw_path.trim_matches('\0').trim().to_lowercase().replace('/', "\\");

        // Strip NT namespace prefixes
        if p.starts_with("\\??\\") {
            p = p[4..].to_string();
        } else if p.starts_with("\\\\?\\") {
            p = p[4..].to_string();
        }

        // Relative path starting with .\ -> strip
        if p.starts_with(".\\") {
            p = p[2..].to_string();
        }

        // Check symlinks
        for (link, target) in &self.symlinks {
            if p.starts_with(link) {
                let rest = &p[link.len()..];
                let rest_clean = rest.trim_start_matches('\\');
                if rest_clean.is_empty() {
                    return target.clone();
                } else {
                    return format!("{}\\{}", target, rest_clean);
                }
            }
        }

        p
    }

    /// Resolves normalized path to a mounted device and the subpath relative to that device.
    pub fn resolve_device<'a>(&'a mut self, path: &str) -> Option<(&'a mut Box<dyn VfsDevice>, String)> {
        let norm = self.normalize_path(path);
        for (prefix, dev) in self.devices.iter_mut() {
            if norm == *prefix {
                return Some((dev, String::new()));
            }
            if norm.starts_with(prefix.as_str()) {
                let rest = &norm[prefix.len()..];
                if rest.starts_with('\\') {
                    return Some((dev, rest.trim_start_matches('\\').to_string()));
                }
            }
        }
        None
    }

    /// Read whole file data from any matching device or fallback.
    pub fn read_file(&self, path: &str) -> Option<Vec<u8>> {
        let norm = self.normalize_path(path);
        for (prefix, dev) in &self.devices {
            if norm.starts_with(prefix) {
                let rest = &norm[prefix.len()..];
                let rel = rest.trim_start_matches('\\');
                if let Ok(data) = dev.read_file(rel) {
                    return Some(data);
                }
            }
        }
        // Fallback: check device with empty prefix or match just by filename in devices
        let fname = norm.rsplit('\\').next().unwrap_or(&norm);
        for (_, dev) in &self.devices {
            if let Ok(data) = dev.read_file(fname) {
                return Some(data);
            }
        }
        None
    }

    /// Writes data to a file on the target device.
    pub fn write_file(&mut self, path: &str, data: &[u8]) -> Result<(), VfsError> {
        if let Some((dev, rel)) = self.resolve_device(path) {
            dev.write_file(&rel, data)
        } else {
            // Default to writing to primary harddisk
            if let Some((_, dev)) = self.devices.iter_mut().find(|(p, _)| p.contains("harddisk0")) {
                let clean = path.trim_matches(['/', '\\']);
                dev.write_file(clean, data)
            } else {
                Err(VfsError::NotFound)
            }
        }
    }

    /// Checks if a file exists.
    pub fn exists(&self, path: &str) -> bool {
        let norm = self.normalize_path(path);
        for (prefix, dev) in &self.devices {
            if norm.starts_with(prefix) {
                let rest = &norm[prefix.len()..];
                let rel = rest.trim_start_matches('\\');
                if dev.exists(rel) {
                    return true;
                }
            }
        }
        let fname = norm.rsplit('\\').next().unwrap_or(&norm);
        for (_, dev) in &self.devices {
            if dev.exists(fname) {
                return true;
            }
        }
        false
    }

    /// Lists directory entries on the target device.
    pub fn list_dir(&self, path: &str) -> Result<Vec<String>, VfsError> {
        let norm = self.normalize_path(path);
        for (prefix, dev) in &self.devices {
            if norm == *prefix {
                return dev.list_dir("");
            }
            if norm.starts_with(prefix) {
                let rest = &norm[prefix.len()..];
                let rel = rest.trim_start_matches('\\');
                return dev.list_dir(rel);
            }
        }
        Err(VfsError::NotFound)
    }
}

// -------------------------------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_directory_is_read_only_and_case_insensitive() {
        let root = std::env::temp_dir().join(format!("xenon_vfs_test_{}", std::process::id()));
        std::fs::create_dir_all(root.join("Sub")).unwrap();
        std::fs::write(root.join("xenonjklatin.xtt"), b"font").unwrap();
        std::fs::write(root.join("Sub").join("shrdres.xzp"), b"XUIZ").unwrap();
        let mut vfs = VirtualFileSystem::new();
        vfs.mount("\\device\\flash", Box::new(HostDirectoryDevice::new("flash", &root)));
        assert_eq!(vfs.read_file("media:\\XenonJKLatin.xtt").as_deref(), Some(&b"font"[..]));
        assert_eq!(vfs.read_file("\\Device\\Flash\\sub\\SHRDRES.xzp").as_deref(), Some(&b"XUIZ"[..]));
        assert!(vfs.exists("flash:\\sub"));
        let (dev, rel) = vfs.resolve_device("media:\\xenonjklatin.xtt").unwrap();
        let mut file = dev.open(&rel, OpenMode::Read).unwrap();
        let mut buf = [0u8; 2];
        file.seek(SeekFrom::Start(2)).unwrap();
        assert_eq!((file.read(&mut buf).unwrap(), &buf), (2, b"nt"));
        assert_eq!(dev.write_file(&rel, b"x"), Err(VfsError::PermissionDenied));
        assert!(dev.open(&rel, OpenMode::ReadWrite).is_err());
        assert_eq!(dev.read_file("missing.xtt"), Err(VfsError::NotFound));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_vfs_path_normalization_and_symlinks() {
        let vfs = VirtualFileSystem::new();
        assert_eq!(
            vfs.normalize_path("hdd0:\\system\\dash.xex"),
            "\\device\\harddisk0\\partition1\\system\\dash.xex"
        );
        assert_eq!(
            vfs.normalize_path("game:/default.xex"),
            "\\device\\harddisk0\\partition1\\game\\default.xex"
        );
        assert_eq!(
            vfs.normalize_path("\\??\\dvd:\\media\\audio.xma"),
            "\\device\\cdrom0\\media\\audio.xma"
        );
    }

    #[test]
    fn test_vfs_memory_device_read_write() {
        let mut vfs = VirtualFileSystem::new();
        let test_data = b"Xbox 360 Dashboard Skin Data";

        vfs.write_file("hdd0:\\skin.xur", test_data).expect("write file");
        assert!(vfs.exists("hdd0:\\skin.xur"));
        assert!(vfs.exists("\\device\\harddisk0\\partition1\\skin.xur"));

        let read_back = vfs.read_file("hdd0:\\skin.xur").expect("read file");
        assert_eq!(read_back, test_data);
    }

    #[test]
    fn test_vfs_memory_file_seek_and_read() {
        let data = b"0123456789ABCDEF".to_vec();
        let mut file = MemoryFile::new(data);

        assert_eq!(file.size(), 16);
        let mut buf = [0u8; 4];
        file.read(&mut buf).unwrap();
        assert_eq!(&buf, b"0123");

        file.seek(SeekFrom::Start(10)).unwrap();
        file.read(&mut buf).unwrap();
        assert_eq!(&buf, b"ABCD");
    }

    #[test]
    fn test_vfs_stfs_package_mounting() {
        if let Ok(su_bytes) = std::fs::read("../$SystemUpdate/su20076000_00000000") {
            let mut vfs = VirtualFileSystem::new();
            let stfs_dev = StfsDevice::new("update", su_bytes).expect("parse STFS package");
            vfs.mount("\\device\\cdrom0\\update", Box::new(stfs_dev));
            vfs.add_symlink("update:", "\\device\\cdrom0\\update");

            // Verify package files are detected via symlink
            let files = vfs.list_dir("update:").unwrap_or_default();
            assert!(!files.is_empty(), "STFS entries should be listed");
        }
    }
}
