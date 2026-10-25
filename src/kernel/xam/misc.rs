//! xam.xex HLE: Everything else exported by xam.xex (locale, cache, files, console settings, ...).

use super::super::dispatcher::{KernelDispatcher, VfsFileHandle};
use super::super::util::read_c_string;
use crate::kernel::objects::ObjectKind;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_misc(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x01C5: XamGetDefaultSystemImage (phImage)
            0x01C5 => {
                if let Some(&p_img) = args.first() {
                    if p_img != 0 {
                        let _ = bus.write_u32(p_img, 0x0000_0001);
                    }
                }
                0
            }

            // 0x070D: XamQueryLiveHiveW (name, output, size, type) - the Live registry hive has no entries offline
            0x070D => 0x0000_0490, // ERROR_NOT_FOUND

            // 0x02B4: XamCacheOpenFile
            0x02B4 => {
                let h = self.new_handle(ObjectKind::File);
                if let Some(&p_h) = args.get(3) {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, h);
                    }
                }
                0
            }

            // 0x02B5: XamCacheCloseFile
            0x02B5 => 0,

            // 0x02B6: XamGetCachedTitleName
            0x02B6 => {
                if let Some(&p_buf) = args.get(1) {
                    if p_buf != 0 {
                        let title = "Xbox 360 Dashboard\0";
                        for (i, c) in title.encode_utf16().enumerate() {
                            let _ = bus.write_u16(p_buf + (i * 2) as u64, c);
                        }
                    }
                }
                0
            }

            // 0x02B9: XamGetCachedGamerTagW
            0x02B9 => {
                if let Some(&p_buf) = args.get(1) {
                    if p_buf != 0 {
                        let tag = "Xbox360\0";
                        for (i, c) in tag.encode_utf16().enumerate() {
                            let _ = bus.write_u16(p_buf + (i * 2) as u64, c);
                        }
                    }
                }
                0
            }

            // 0x03CB: XGetAVPack
            0x03CB => 6, // XAVPACK_HDMI

            // 0x03D1: XGetVideoMode
            0x03D1 => {
                if let Some(&p_mode) = args.first() {
                    if p_mode != 0 {
                        let _ = bus.write_u32(p_mode + 0, 1280);
                        let _ = bus.write_u32(p_mode + 4, 720);
                        let _ = bus.write_u32(p_mode + 8, 0);
                        let _ = bus.write_u32(p_mode + 12, 1);
                        let _ = bus.write_u32(p_mode + 16, 60);
                    }
                }
                0
            }

            // 0x0417: SetFilePointer (hFile, lDistanceToMove, lpDistanceToMoveHigh, dwMoveMethod)
            0x0417 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let dist = args.get(1).copied().unwrap_or(0) as i32;
                let method = args.get(3).copied().unwrap_or(0) as u32;
                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    let new_pos = match method {
                        0 => dist.max(0) as usize, // FILE_BEGIN
                        1 => (file.position as i64 + dist as i64).max(0) as usize, // FILE_CURRENT
                        2 => (file.data.len() as i64 + dist as i64).max(0) as usize, // FILE_END
                        _ => file.position,
                    };
                    file.position = new_pos;
                    file.position as u64
                } else {
                    0xFFFF_FFFF
                }
            }

            // 0x041C: ReadFile (hFile, lpBuffer, nNumberOfBytesToRead, lpNumberOfBytesRead, lpOverlapped)
            0x041C => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_buf = args.get(1).copied().unwrap_or(0);
                let len = args.get(2).copied().unwrap_or(0) as usize;
                let p_read = args.get(3).copied().unwrap_or(0);
                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    let avail = file.data.len().saturating_sub(file.position);
                    let to_read = std::cmp::min(len, avail);
                    if to_read > 0 && p_buf != 0 {
                        let slice = &file.data[file.position..file.position + to_read];
                        let _ = bus.load_bytes(p_buf, slice);
                        file.position += to_read;
                    }
                    if p_read != 0 {
                        let _ = bus.write_u32(p_read, to_read as u32);
                    }
                    1 // TRUE
                } else {
                    0 // FALSE
                }
            }

            // 0x0427: GetFileSize (hFile, lpFileSizeHigh)
            0x0427 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_high = args.get(1).copied().unwrap_or(0);
                if let Some(file) = self.vfs_handles.get(&h) {
                    if p_high != 0 {
                        let _ = bus.write_u32(p_high, 0);
                    }
                    file.data.len() as u64
                } else {
                    0xFFFF_FFFF
                }
            }

            // 0x0447: CreateFileA (lpFileName, dwDesiredAccess, dwShareMode, lpSecurityAttributes, dwCreationDisposition, ...)
            0x0447 => {
                let p_name = args.first().copied().unwrap_or(0);
                if let Some(path) = read_c_string(p_name, bus) {
                    println!("    [VFS] CreateFileA: '{}'", path);
                    if let Some(data) = self.find_vfs_file(&path) {
                        let h = self.new_handle(ObjectKind::File);
                        self.vfs_handles.insert(h, VfsFileHandle {
                            handle: h,
                            name: path.clone(),
                            data,
                            position: 0,
                            is_directory: false,
                        });
                        h as u64
                    } else {
                        0xFFFF_FFFF_FFFF_FFFF // INVALID_HANDLE_VALUE
                    }
                } else {
                    0xFFFF_FFFF_FFFF_FFFF
                }
            }

            // 0x04C1: XamFitnessClearBodyProfileRecords
            0x04C1 => 0,

            // 0x04E4: XamBackgroundDownloadNetworkStorageRegisterChangeCallback
            0x04E4 => 0,

            // 0x050B: XamGetConsoleFriendlyName
            0x050B => {
                if let Some(&p_buf) = args.first() {
                    if p_buf != 0 {
                        let name = "Xbox 360\0";
                        for (i, c) in name.encode_utf16().enumerate() {
                            let _ = bus.write_u16(p_buf + (i * 2) as u64, c);
                        }
                    }
                }
                0
            }

            // 0x01CB (459): XamGetOnlineSchema() -> const struct { const void *data; uint32_t size; } *
            // The dash passes the returned pointer straight into its schema parser (0x9224AA48), which fails with
            // ERROR_INVALID_PARAMETER (0x57) on NULL and aborts the whole service initialisation. We return a minimal,
            // valid schema: a 44-byte header (all table counts 0, flags clear) whose total size is stored at +20.
            0x01CB => {
                if self.online_schema_ptr == 0 {
                    const SCHEMA_SIZE: u32 = 0x2C;
                    let blob = self.valloc_bump;
                    self.valloc_bump += 0x100;
                    let st = self.valloc_bump;
                    self.valloc_bump += 0x10;
                    for i in 0..0x100u64 { let _ = bus.write_u8(blob + i, 0); }
                    let _ = bus.write_u32(blob + 20, SCHEMA_SIZE); // total schema size
                    let _ = bus.write_u32(st, blob as u32);       // data*
                    let _ = bus.write_u32(st + 4, SCHEMA_SIZE);   // size
                    self.online_schema_ptr = st;
                    println!("    -> XamGetOnlineSchema: built minimal schema struct @0x{:08X} (data @0x{:08X}, size 0x{:X})", st, blob, SCHEMA_SIZE);
                }
                self.online_schema_ptr
            }

            // 0x050C: XamSetConsoleFriendlyName
            0x050C => 0,

            // 0x0282: XamGetSystemVersion() -> 0 (Xenia xam_info.cc stub)
            0x0282 => 0,

            // 0x0650: XdfInitialize
            0x0650 => 0,

            // 0x076F: XamGetStagingMode
            0x076F => 0,

            // 0x07DC: XamGetDashBackstackData
            0x07DC => 0,

            // 0x09C4: XamBackgroundDownloadSetMode
            0x09C4 => 0,

            // 0x0B43: XamSetCountry
            0x0B43 => 0,

            _ => return None,
        })
    }
}
