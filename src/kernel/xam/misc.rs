//! xam.xex HLE: Everything else exported by xam.xex (locale, cache, files, console settings, ...).

use super::super::dispatcher::{KernelDispatcher, VfsFileHandle};
use super::super::util::read_c_string;
use crate::kernel::objects::ObjectKind;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_misc(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x059F: XamLogLocalizationEtx(error_code, unk) -> success for 0x80300034/0x80300035, else
            // X_E_NOT_IMPLEMENTED (Xenia xam_info.cc stub)
            0x059F => match args.first().copied().unwrap_or(0) as u32 {
                0x8030_0034 | 0x8030_0035 => 0,
                _ => 0x8000_4001,
            },

            // 0x087A: XamIsNuiAutomationEnabled(unk1, unk2) -> X_ERROR_SUCCESS (Xenia xam_nui.cc stub)
            0x087A => 0,

            // 0x05DC: XamAvatarInitialize(...) -> ~0: avatar initialization is off outside the avatar editor
            // (Xenia cvar allow_avatar_initialization = false)
            0x05DC => 0xFFFF_FFFF,

            // 0x05E7: XamAvatarLoadAnimation(asset_id_ptr, flags, output, overlapped): completes at once; with an
            // overlapped returns ERROR_IO_PENDING (Xenia xam_avatar.cc)
            0x05E7 => {
                let overlapped = args.get(3).copied().unwrap_or(0);
                if overlapped != 0 {
                    self.complete_overlapped_immediate(bus, overlapped, 0);
                    997 // X_ERROR_IO_PENDING
                } else {
                    0
                }
            }

            // 0x058C: XamGetLanguageTypeface(language, buffer_size, buffer): the typeface locator of the language
            // (Xenia xam_locale.cc)
            0x058C => {
                let path = match args.first().copied().unwrap_or(0) {
                    7 => "file://media:/XenonSCLatin.xtt", // XLanguage::kSChinese
                    8 => "file://media:/XenonCLatin.xtt",  // XLanguage::kTChinese
                    _ => "file://media:/XenonJKLatin.xtt",
                };
                Self::write_utf16_truncating(bus, args.get(2).copied().unwrap_or(0), path, path.len() + 1);
                0
            }

            // 0x0315: XamBuildSharedSystemResourceLocator(filename, buffer, buffer_count bytes):
            // "section://<xam hmodule>,shrdres#<filename>" (Xenia xam_info.cc keXamBuildResourceLocator)
            0x0315 => {
                let name = Self::read_utf16(bus, args.first().copied().unwrap_or(0));
                let xam = self.modules.modules.iter().find(|m| m.name.eq_ignore_ascii_case("xam.xex")).map(|m| m.hmodule).unwrap_or(0);
                let path = if xam != 0 { format!("section://{:X},shrdres#{}", xam, name) } else { format!("file://media:/shrdres.xzp#{}", name) };
                Self::write_utf16_truncating(bus, args.get(1).copied().unwrap_or(0), &path, (args.get(2).copied().unwrap_or(0) / 2) as usize);
                0
            }

            // 0x03CA: XGetAudioFlags() -> XConfig audio flags, Dolby Digital | Dolby Pro Logic by default
            0x03CA => 0x0001_0001,

            // 0x0200: XamFeatureEnabled(feature_bit) -> bit set in 0x40FFFFFFFF (Xenia xam_info.cc)
            0x0200 => {
                let bit = args.first().copied().unwrap_or(0);
                (bit < 36 && (0x40_FFFF_FFFFu64 >> bit) & 1 != 0) as u64
            }

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

            // 0x0415: GetTickCount() -> ms count
            0x0415 => {
                let ticks = self.host.time().query_performance_counter();
                (ticks / 50_000) as u64
            }

            // 0x042F: QueryPerformanceCounter (lpPerformanceCount) -> BOOL
            0x042F => {
                let p_counter = args.first().copied().unwrap_or(0);
                if p_counter != 0 {
                    let ticks = self.host.time().query_performance_counter();
                    let _ = bus.write_u64(p_counter, ticks);
                }
                1 // TRUE
            }

            // 0x0430: QueryPerformanceFrequency (lpFrequency) -> BOOL
            0x0430 => {
                let p_freq = args.first().copied().unwrap_or(0);
                if p_freq != 0 {
                    let freq = self.host.time().query_performance_frequency();
                    let _ = bus.write_u64(p_freq, freq);
                }
                1 // TRUE
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

            // 0x03D2: XamGetLanguage() -> the XConfig user language (kernel/xconfig.rs)
            0x03D2 => crate::kernel::xconfig::language() as u64,

            // 0x04A9: XamGetLocale() -> the locale of the XConfig user country
            0x04A9 => crate::kernel::xconfig::locale() as u64,

            // 0x058B: XamGetOnlineLanguageAndCountry(xuid, language_buffer, country_buffer) -> void: one byte each,
            // from the XConfig user settings (the signed-in profile has no settings of its own here)
            0x058B => {
                if let Some(&p_lang) = args.get(1) {
                    if p_lang != 0 {
                        let _ = bus.write_u8(p_lang, crate::kernel::xconfig::USER_LANGUAGE as u8);
                    }
                }
                if let Some(&p_country) = args.get(2) {
                    if p_country != 0 {
                        let _ = bus.write_u8(p_country, crate::kernel::xconfig::USER_COUNTRY);
                    }
                }
                0
            }

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

#[cfg(test)]
mod tests {
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    fn xam(k: &mut KernelDispatcher, bus: &mut MemoryBus, ordinal: u16, args: &[u64]) -> u64 {
        k.dispatch("xam.xex", ordinal, args, bus)
    }

    #[test]
    fn exports_added_from_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        assert_eq!(xam(&mut k, &mut bus, 0x087A, &[0, 0]), 0);
        assert_eq!((xam(&mut k, &mut bus, 0x059F, &[0x8030_0034, 0]), xam(&mut k, &mut bus, 0x059F, &[1, 0])), (0, 0x8000_4001));
        assert_eq!(xam(&mut k, &mut bus, 0x05DC, &[1, 0, 0, 0, 0, 0]), 0xFFFF_FFFF);
        assert_eq!(xam(&mut k, &mut bus, 0x03CA, &[]), 0x0001_0001);
        assert_eq!((xam(&mut k, &mut bus, 0x0200, &[0]), xam(&mut k, &mut bus, 0x0200, &[32]), xam(&mut k, &mut bus, 0x0200, &[38])), (1, 0, 0), "std::bitset<36> drops bit 38 of the constant");
        // NetDll_inet_addr
        bus.load_bytes(0x4100_0000, b"192.168.1.20\0").unwrap();
        assert_eq!(xam(&mut k, &mut bus, 0x001A, &[0x4100_0000]), 0xC0A8_0114);
        bus.write_u8(0x4100_0000, 0).unwrap();
        assert_eq!(xam(&mut k, &mut bus, 0x001A, &[0x4100_0000]), 0);
        // XamGetLanguageTypeface(English): "file://media:/XenonJKLatin.xtt"
        xam(&mut k, &mut bus, 0x058C, &[1, 0x100, 0x4100_0100]);
        assert_eq!(KernelDispatcher::read_utf16(&bus, 0x4100_0100), "file://media:/XenonJKLatin.xtt");
        // XamBuildSharedSystemResourceLocator without a registered xam module: the local file form
        for (i, u) in "fonts".encode_utf16().chain([0]).enumerate() {
            bus.write_u16(0x4100_0200 + 2 * i as u64, u).unwrap();
        }
        xam(&mut k, &mut bus, 0x0315, &[0x4100_0200, 0x4100_0300, 0x80]);
        assert_eq!(KernelDispatcher::read_utf16(&bus, 0x4100_0300), "file://media:/shrdres.xzp#fonts");
        // XamUserIsUnsafeProgrammingAllowed
        assert_eq!(xam(&mut k, &mut bus, 0x02AE, &[0, 0, 0x4100_0400]), 0);
        assert_eq!(bus.read_u32(0x4100_0400).unwrap(), 1);
        assert_eq!(xam(&mut k, &mut bus, 0x02AE, &[5, 0, 0x4100_0400]), 0x57);
        // XamAvatarLoadAnimation with an overlapped: completed at once, its event set, ERROR_IO_PENDING
        k.dispatch("xboxkrnl.exe", 0x00D1, &[0x4100_0500, 0, 0, 0], &mut bus);
        let ev = bus.read_u32(0x4100_0500).unwrap();
        bus.write_u32(0x4100_060C, ev).unwrap();
        assert_eq!(xam(&mut k, &mut bus, 0x05E7, &[0x4100_0700, 0, 0, 0x4100_0600]), 997);
        let ev_ptr = k.objects.lookup(ev, 0).unwrap().guest_ptr as u64;
        assert_eq!((bus.read_u32(0x4100_0600).unwrap(), bus.read_u32(ev_ptr + 4).unwrap()), (0, 1));
        // callbacks are stored
        xam(&mut k, &mut bus, 0x07E2, &[0x9220_0000]);
        xam(&mut k, &mut bus, 0x0289, &[0x9230_0000]);
        assert_eq!((k.launch_request_callback, k.content_change_callback), (0x9220_0000, 0x9230_0000));
        // kernel side: ObReferenceObjectByName finds nothing; KeQueryInterruptTime follows the time base
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0111, &[0, 0, 0, 0, 0x4100_0800], &mut bus), 0xC000_0034);
        bus.timebase.begin_frame(6, 1);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0082, &[], &mut bus), 6 * 833_333 / 5);

        // QueryPerformanceFrequency & QueryPerformanceCounter & GetTickCount
        assert_eq!(xam(&mut k, &mut bus, 0x0430, &[0x4100_0900]), 1);
        assert_eq!(bus.read_u64(0x4100_0900).unwrap(), 50_000_000);
        assert_eq!(xam(&mut k, &mut bus, 0x042F, &[0x4100_0908]), 1);
        let qpc1 = bus.read_u64(0x4100_0908).unwrap();
        assert!(qpc1 > 0);
        assert_eq!(xam(&mut k, &mut bus, 0x042F, &[0x4100_0910]), 1);
        let qpc2 = bus.read_u64(0x4100_0910).unwrap();
        assert!(qpc2 > qpc1);
        let tc = xam(&mut k, &mut bus, 0x0415, &[]);
        assert!(tc > 0);
    }
}
