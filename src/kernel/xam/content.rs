//! xam.xex HLE: XamContent*/enumerators: content packages.

use super::super::dispatcher::{KernelDispatcher, VfsFileHandle};
use super::super::util::read_c_string;
use crate::kernel::objects::ObjectKind;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xam_content(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x0250: XamEnumerate
            0x0250 => 0x0000_0103, // STATUS_NO_MORE_FILES

            // 0x0259: XamContentCreateEx
            0x0259 => 0,

            // 0x0269: XamContentOpenFile | 0x0278: XamContentOpenFileInternal | 0x027B: XamContentOpenPackageFile
            0x0269 | 0x0278 | 0x027B => {
                let p_path = args.get(2).copied().unwrap_or(0);
                let p_handle = args.get(4).copied().unwrap_or(0);
                if let Some(path) = read_c_string(p_path, bus) {
                    println!("    [VFS] XamContentOpenFile: '{}'", path);
                    if let Some(data) = self.find_vfs_file(&path) {
                        let h = self.new_handle(ObjectKind::File);
                        self.vfs_handles.insert(h, VfsFileHandle {
                            handle: h,
                            name: path.clone(),
                            data,
                            position: 0,
                            is_directory: false,
                        });
                        if p_handle != 0 {
                            let _ = bus.write_u32(p_handle, h);
                        }
                        0
                    } else {
                        0xC000_0034
                    }
                } else {
                    0xC000_000D
                }
            }

            // 0x0276: XamContentResolveInternal (userIndex, flags, pPathBuffer, cchBuffer, ...)
            0x0276 => {
                if let Some(&p_buf) = args.get(2) {
                    if p_buf != 0 {
                        let path = b"game:\\\0";
                        for (i, &b) in path.iter().enumerate() {
                            let _ = bus.write_u8(p_buf + i as u64, b);
                        }
                    }
                }
                println!("    -> XamContentResolveInternal: Resolved content root path to 'game:\\'");
                0 // ERROR_SUCCESS
            }

            _ => return None,
        })
    }
}
