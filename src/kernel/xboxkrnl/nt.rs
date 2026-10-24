//! xboxkrnl.exe HLE: Nt*/Ob*/Io*: system services, object manager and I/O.

use super::super::dispatcher::{KernelDispatcher, KernelEvent, VfsFileHandle};
use super::super::util::read_vfs_path;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_nt(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x00CC: NtAllocateVirtualMemory
            // (IN OUT PVOID *BaseAddress, IN OUT PSIZE_T RegionSize, ULONG AllocationType, ULONG Protect, ULONG ZeroBits)
            0x00CC => {
                let p_base = args.get(0).copied().unwrap_or(0);
                let p_size = args.get(1).copied().unwrap_or(0);
                let alloc_type = args.get(2).copied().unwrap_or(0);
                let protect = args.get(3).copied().unwrap_or(0);

                let cur_base = if p_base != 0 {
                    bus.read_u32(p_base).unwrap_or(0) as u64
                } else {
                    0
                };

                let req_size = if p_size != 0 {
                    bus.read_u32(p_size).unwrap_or(0x1000) as u64
                } else {
                    0x1000
                };

                let aligned_size = (req_size.max(0x1000) + 0xFFF) & !0xFFF;

                let target_addr = if cur_base != 0 {
                    cur_base
                } else {
                    let addr = self.valloc_bump;
                    self.valloc_bump += aligned_size;
                    addr
                };

                if p_base != 0 {
                    let _ = bus.write_u32(p_base, target_addr as u32);
                }
                if p_size != 0 {
                    let _ = bus.write_u32(p_size, aligned_size as u32);
                }

                println!(
                    "    -> NtAllocateVirtualMemory: Base=0x{:08X}, Size=0x{:X} (Type=0x{:X}, Prot=0x{:X})",
                    target_addr, aligned_size, alloc_type, protect
                );
                0 // STATUS_SUCCESS
            }

            // 0x00CE: NtClearEvent
            0x00CE => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        ev.signaled = false;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 0);
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00CF: NtClose (Handle)
            0x00CF => {
                let h = args.first().copied().unwrap_or(0) as u32;
                self.vfs_handles.remove(&h);
                self.events.remove(&h);
                0 // STATUS_SUCCESS
            }

            // 0x00D1: NtCreateEvent (pHandle, desiredAccess, pObjAttr, eventType, initialState)
            0x00D1 => {
                let h = self.next_handle;
                self.next_handle += 4;
                let event_type = args.get(3).copied().unwrap_or(0) as u32;
                let initial = args.get(4).copied().unwrap_or(0) as u32;

                if let Some(&p_handle) = args.first() {
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h);
                    }
                }
                self.events.insert(h, KernelEvent {
                    handle: h,
                    event_type,
                    signaled: initial != 0,
                    guest_ptr: 0,
                });
                0 // STATUS_SUCCESS
            }

            // 0x0038: IoCreateFile | 0x00D2: NtCreateFile | 0x00DF: NtOpenFile
            0x0038 | 0x00D2 | 0x00DF => {
                let p_handle = args.first().copied().unwrap_or(0);
                let p_obj_attr = args.get(2).copied().unwrap_or(0);
                let p_io_status = args.get(3).copied().unwrap_or(0);

                let path_opt = read_vfs_path(p_obj_attr, bus);
                if let Some(path) = path_opt {
                    println!("    [VFS] NtCreateFile / NtOpenFile: '{}'", path);
                    if let Some(data) = self.find_vfs_file(&path) {
                        let h = self.next_handle;
                        self.next_handle += 4;
                        let is_dir = path.ends_with('\\') || path.ends_with('/');
                        self.vfs_handles.insert(h, VfsFileHandle {
                            handle: h,
                            name: path.clone(),
                            data,
                            position: 0,
                            is_directory: is_dir,
                        });
                        if p_handle != 0 {
                            let _ = bus.write_u32(p_handle, h);
                        }
                        if p_io_status != 0 {
                            let _ = bus.write_u32(p_io_status, 0); // STATUS_SUCCESS
                            let _ = bus.write_u32(p_io_status + 4, 1); // FILE_OPENED
                        }
                        println!("      -> VFS Handle 0x{:X} granted for '{}'", h, path);
                        0 // STATUS_SUCCESS
                    } else {
                        println!("      -> VFS File NOT found: '{}'", path);
                        if p_io_status != 0 {
                            let _ = bus.write_u32(p_io_status, 0xC000_0034); // STATUS_OBJECT_NAME_NOT_FOUND
                            let _ = bus.write_u32(p_io_status + 4, 0);
                        }
                        0xC000_0034 // STATUS_OBJECT_NAME_NOT_FOUND
                    }
                } else {
                    let h = self.next_handle;
                    self.next_handle += 4;
                    self.vfs_handles.insert(h, VfsFileHandle {
                        handle: h,
                        name: "<unnamed>".to_string(),
                        data: Vec::new(),
                        position: 0,
                        is_directory: false,
                    });
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h);
                    }
                    if p_io_status != 0 {
                        let _ = bus.write_u32(p_io_status, 0);
                        let _ = bus.write_u32(p_io_status + 4, 1);
                    }
                    0 // STATUS_SUCCESS
                }
            }

            // 0x00D3: NtCreateIoCompletion
            0x00D3 => {
                let handle = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.first() {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, handle);
                    }
                }
                0
            }

            // 0x00D5: NtCreateSemaphore
            0x00D5 => {
                let handle = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.first() {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, handle);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00D8: NtDeleteFile
            0x00D8 => 0,

            // 0x00D9: NtDeviceIoControlFile
            0x00D9 => {
                if let Some(&p_ios) = args.get(4) {
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 0);
                    }
                }
                0
            }

            // 0x00DB: NtFlushBuffersFile
            0x00DB => 0,

            // 0x00DC: NtFreeVirtualMemory
            0x00DC => 0,

            // 0x00E2: NtPulseEvent
            0x00E2 => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        ev.signaled = false;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 0);
                        }
                    }
                }
                0
            }

            // 0x00E7: NtQueryFullAttributesFile (pObjAttr, pNetworkOpenInfo)
            0x00E7 => {
                let p_obj_attr = args.first().copied().unwrap_or(0);
                let p_info = args.get(1).copied().unwrap_or(0);
                if let Some(path) = read_vfs_path(p_obj_attr, bus) {
                    if let Some(data) = self.find_vfs_file(&path) {
                        if p_info != 0 {
                            let flen = data.len() as u64;
                            let _ = bus.write_u64(p_info + 0, 0); // CreationTime
                            let _ = bus.write_u64(p_info + 8, 0); // LastAccessTime
                            let _ = bus.write_u64(p_info + 16, 0); // LastWriteTime
                            let _ = bus.write_u64(p_info + 24, 0); // ChangeTime
                            let _ = bus.write_u64(p_info + 32, flen); // AllocationSize
                            let _ = bus.write_u64(p_info + 40, flen); // EndOfFile
                            let _ = bus.write_u32(p_info + 48, 0x80); // FILE_ATTRIBUTE_NORMAL
                        }
                        0 // STATUS_SUCCESS
                    } else {
                        0xC000_0034 // STATUS_OBJECT_NAME_NOT_FOUND
                    }
                } else {
                    0xC000_000D // STATUS_INVALID_PARAMETER
                }
            }

            // 0x00E8: NtQueryInformationFile (FileHandle, IoStatusBlock, FileInformation, Length, FileInformationClass)
            0x00E8 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_ios = args.get(1).copied().unwrap_or(0);
                let p_info = args.get(2).copied().unwrap_or(0);
                let info_class = args.get(4).copied().unwrap_or(0) as u32;

                if let Some(file) = self.vfs_handles.get(&h) {
                    let flen = file.data.len() as u64;
                    let pos = file.position as u64;
                    match info_class {
                        5 => { // FileStandardInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, flen); // AllocationSize
                                let _ = bus.write_u64(p_info + 8, flen); // EndOfFile (size)
                                let _ = bus.write_u32(p_info + 16, 1);   // NumberOfLinks
                                let _ = bus.write_u8(p_info + 20, 0);    // DeletePending
                                let _ = bus.write_u8(p_info + 21, if file.is_directory { 1 } else { 0 }); // Directory
                            }
                        }
                        14 => { // FilePositionInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, pos); // CurrentByteOffset
                            }
                        }
                        4 => { // FileBasicInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, 0);
                                let _ = bus.write_u64(p_info + 8, 0);
                                let _ = bus.write_u64(p_info + 16, 0);
                                let _ = bus.write_u64(p_info + 24, 0);
                                let _ = bus.write_u32(p_info + 32, 0x80);
                            }
                        }
                        21 => { // FileNetworkOpenInformation
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, 0);
                                let _ = bus.write_u64(p_info + 8, 0);
                                let _ = bus.write_u64(p_info + 16, 0);
                                let _ = bus.write_u64(p_info + 24, 0);
                                let _ = bus.write_u64(p_info + 32, flen);
                                let _ = bus.write_u64(p_info + 40, flen);
                                let _ = bus.write_u32(p_info + 48, 0x80);
                            }
                        }
                        _ => {
                            if p_info != 0 {
                                let _ = bus.write_u64(p_info + 0, flen);
                                let _ = bus.write_u64(p_info + 8, flen);
                            }
                        }
                    }
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 24);
                    }
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00EE: NtQueryVirtualMemory
            0x00EE => 0,

            // 0x00EF: NtQueryVolumeInformationFile
            0x00EF => {
                if let Some(&p_ios) = args.get(1) {
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 0);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00F0: NtReadFile (FileHandle, Event, ApcRoutine, ApcContext, IoStatusBlock, Buffer, Length, ByteOffset, Key)
            0x00F0 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let event_h = args.get(1).copied().unwrap_or(0) as u32;
                let p_ios = args.get(4).copied().unwrap_or(0);
                let p_buf = args.get(5).copied().unwrap_or(0);
                let len = args.get(6).copied().unwrap_or(0) as usize;
                let p_offset = args.get(7).copied().unwrap_or(0);

                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    if p_offset != 0 {
                        let off = bus.read_u64(p_offset).unwrap_or(0) as usize;
                        file.position = off;
                    }
                    if file.position >= file.data.len() && len > 0 {
                        if p_ios != 0 {
                            let _ = bus.write_u32(p_ios, 0xC000_0011); // STATUS_END_OF_FILE
                            let _ = bus.write_u32(p_ios + 4, 0);
                        }
                        return Some(0xC000_0011);
                    }
                    let avail = file.data.len().saturating_sub(file.position);
                    let to_read = std::cmp::min(len, avail);
                    if to_read > 0 && p_buf != 0 {
                        let slice = &file.data[file.position..file.position + to_read];
                        let _ = bus.load_bytes(p_buf, slice);
                        file.position += to_read;
                    }
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, to_read as u32);
                    }
                    if event_h != 0 {
                        if let Some(ev) = self.events.get_mut(&event_h) {
                            ev.signaled = true;
                            if ev.guest_ptr != 0 {
                                let _ = bus.write_u32(ev.guest_ptr + 4, 1);
                            }
                        }
                    }
                    println!("    [VFS] NtReadFile: read {} bytes from '{}' (pos: {}/{})", to_read, file.name, file.position, file.data.len());
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00F3: NtReleaseSemaphore (hSemaphore, releaseCount, pPreviousCount)
            0x00F3 => {
                if let Some(&p_prev) = args.get(2) {
                    if p_prev != 0 {
                        let _ = bus.write_u32(p_prev, 1);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00F4: NtRemoveIoCompletion
            0x00F4 => 0x0000_0102, // STATUS_TIMEOUT

            // 0x00F6: NtSetEvent (hEvent, pPrevState)
            0x00F6 => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    let prev = if let Some(ev) = self.events.get_mut(&h32) {
                        let p = ev.signaled;
                        ev.signaled = true;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 1);
                        }
                        if p { 1 } else { 0 }
                    } else {
                        0
                    };
                    if let Some(&p_prev) = args.get(1) {
                        if p_prev != 0 {
                            let _ = bus.write_u32(p_prev, prev);
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00F7: NtSetInformationFile (FileHandle, IoStatusBlock, FileInformation, Length, FileInformationClass)
            0x00F7 => {
                let h = args.first().copied().unwrap_or(0) as u32;
                let p_ios = args.get(1).copied().unwrap_or(0);
                let p_info = args.get(2).copied().unwrap_or(0);
                let info_class = args.get(4).copied().unwrap_or(0) as u32;

                if let Some(file) = self.vfs_handles.get_mut(&h) {
                    if info_class == 14 && p_info != 0 {
                        let new_pos = bus.read_u64(p_info).unwrap_or(0) as usize;
                        file.position = new_pos;
                    }
                    if p_ios != 0 {
                        let _ = bus.write_u32(p_ios, 0);
                        let _ = bus.write_u32(p_ios + 4, 0);
                    }
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00F8: NtSetIoCompletion
            0x00F8 => 0,

            // 0x00FB: NtSignalAndWaitForSingleObjectEx
            0x00FB => {
                if let Some(&h1) = args.first() {
                    let h32 = h1 as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        ev.signaled = true;
                        if ev.guest_ptr != 0 {
                            let _ = bus.write_u32(ev.guest_ptr + 4, 1);
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00FC: NtSuspendThread
            0x00FC => 0,

            // 0x00FD: NtWaitForSingleObjectEx
            0x00FD => {
                if let Some(&h) = args.first() {
                    let h32 = h as u32;
                    if let Some(ev) = self.events.get_mut(&h32) {
                        if ev.event_type == 1 {
                            ev.signaled = false;
                            if ev.guest_ptr != 0 {
                                let _ = bus.write_u32(ev.guest_ptr + 4, 0);
                            }
                        }
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x00FE: NtWaitForMultipleObjectsEx
            0x00FE => 0, // STATUS_WAIT_0

            // 0x00FF: NtWriteFile
            0x00FF => 0,

            // 0x0101: NtYieldExecution
            0x0101 => 0,

            // 0x0103: ObCreateSymbolicLink | 0x0104: ObDeleteSymbolicLink | 0x0105: ObDereferenceObject
            0x0103 | 0x0104 | 0x0105 => 0,

            // 0x010B: ObLookupThreadByThreadId
            0x010B => {
                if let Some(&p_obj) = args.get(1) {
                    if p_obj != 0 {
                        let _ = bus.write_u32(p_obj, 0x8001_0000);
                    }
                }
                0
            }

            // 0x010E: ObOpenObjectByPointer
            0x010E => {
                let handle = self.next_handle;
                self.next_handle += 4;
                if let Some(&p_h) = args.get(3) {
                    if p_h != 0 {
                        let _ = bus.write_u32(p_h, handle);
                    }
                }
                0
            }

            // 0x0110: ObReferenceObjectByHandle
            0x0110 => {
                if let Some(&p_obj) = args.get(3) {
                    if p_obj != 0 {
                        let _ = bus.write_u32(p_obj, 0x8001_0000);
                    }
                }
                0
            }

            _ => return None,
        })
    }
}
