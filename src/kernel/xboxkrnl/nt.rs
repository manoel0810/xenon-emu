//! xboxkrnl.exe HLE: Nt*/Ob*/Io*: system services, object manager and I/O.

use super::super::dispatcher::{KernelDispatcher, VfsFileHandle};
use super::super::util::read_vfs_path;
use crate::kernel::objects::{self, KernelObject, ObjectKind};
use crate::kernel::scheduler;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_nt(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(0);
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

            // 0x00CE: NtClearEvent(handle) (xeNtClearEvent)
            0x00CE => match self.handle_object(arg(0), ObjectKind::Event) {
                Ok(ev) => {
                    objects::reset_event(bus, ev);
                    0
                }
                Err(status) => status,
            },

            // 0x00CF: NtClose(handle): releases one handle reference (ObjectTable::ReleaseHandle)
            0x00CF => {
                let h = arg(0) as u32;
                let (status, gone) = self.objects.release(h, self.current_thread);
                if gone.is_some() {
                    self.vfs_handles.remove(&h);
                    self.notifications.listeners.remove(&h);
                }
                status
            }

            // 0x00D1: NtCreateEvent(handle_ptr, obj_attributes, event_type, initial_state): a native KEVENT
            // (XEvent::Initialize); event_type 0 = notification (manual reset), 1 = synchronization. Object names
            // are not looked up yet.
            0x00D1 => {
                let ev = self.create_native(bus, objects::KEVENT_SIZE);
                objects::init_header(bus, ev, (arg(2) != 0) as u8, objects::KEVENT_SIZE, (arg(3) != 0) as u32);
                let h = self.objects.create(KernelObject { kind: ObjectKind::Event, guest_ptr: ev as u32 });
                objects::stash_handle(bus, ev, h);
                if arg(0) != 0 {
                    let _ = bus.write_u32(arg(0), h);
                }
                0
            }

            // 0x00D4: NtCreateMutant(handle_ptr, obj_attributes, initial_owner): a native KMUTANT, owned by the caller
            // if initial_owner (Xenia keeps mutants host-side; waits here read the guest header)
            0x00D4 => {
                let mutant = self.create_native(bus, objects::KMUTANT_SIZE);
                objects::init_mutant(bus, mutant, if arg(2) != 0 { self.current_kthread } else { 0 });
                let h = self.objects.create(KernelObject { kind: ObjectKind::Mutant, guest_ptr: mutant as u32 });
                objects::stash_handle(bus, mutant, h);
                if arg(0) != 0 {
                    let _ = bus.write_u32(arg(0), h);
                }
                0
            }

            // 0x00D7: NtCreateTimer(handle_ptr, obj_attributes, timer_type): a native KTIMER header, not signaled (timer
            // type 0 = notification, 1 = synchronization); timers fire from phase 3.4
            0x00D7 => {
                let timer = self.create_native(bus, objects::KTIMER_SIZE);
                let timer_type = objects::TIMER_NOTIFICATION_OBJECT + (arg(2) & 1) as u8;
                objects::init_header(bus, timer, timer_type, objects::KTIMER_SIZE, 0);
                let h = self.objects.create(KernelObject { kind: ObjectKind::Timer, guest_ptr: timer as u32 });
                objects::stash_handle(bus, timer, h);
                if arg(0) != 0 {
                    let _ = bus.write_u32(arg(0), h);
                }
                0
            }

            // 0x00F2: NtReleaseMutant(handle, previous_count_ptr): releases it if the caller owns it; Xenia returns
            // success either way and does not write the previous count
            0x00F2 => match self.handle_object(arg(0), ObjectKind::Mutant) {
                Ok(mutant) => {
                    let _ = objects::release_mutant(bus, mutant, self.current_kthread);
                    0
                }
                Err(status) => status,
            },

            // 0x00DA: NtDuplicateObject(handle, new_handle_ptr, options); options 1 = DUPLICATE_CLOSE_SOURCE
            0x00DA => {
                let (status, new_handle) = match self.objects.duplicate(arg(0) as u32, self.current_thread) {
                    Ok(h) => (0, h),
                    Err(status) => (status, 0xFFFF_FFFF),
                };
                if arg(1) != 0 {
                    let _ = bus.write_u32(arg(1), new_handle);
                }
                if arg(2) == 1 {
                    self.objects.release(arg(0) as u32, self.current_thread);
                }
                status
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
                        let h = self.new_handle(ObjectKind::File);
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
                    let h = self.new_handle(ObjectKind::File);
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

            // 0x00D3: NtCreateIoCompletion(out_handle, desired_access, obj_attributes, concurrent_threads)
            0x00D3 => {
                let h = self.new_handle(ObjectKind::IoCompletion);
                if arg(0) != 0 {
                    let _ = bus.write_u32(arg(0), h);
                }
                0
            }

            // 0x00D5: NtCreateSemaphore(handle_ptr, obj_attributes, count, limit): a native KSEMAPHORE
            // (XSemaphore::Initialize)
            0x00D5 => {
                let sem = self.create_native(bus, objects::KSEMAPHORE_SIZE);
                objects::init_header(bus, sem, objects::SEMAPHORE_OBJECT, objects::KSEMAPHORE_SIZE, arg(2) as u32);
                let _ = bus.write_u32(sem + objects::KSEMAPHORE_LIMIT, arg(3) as u32);
                let h = self.objects.create(KernelObject { kind: ObjectKind::Semaphore, guest_ptr: sem as u32 });
                objects::stash_handle(bus, sem, h);
                if arg(0) != 0 {
                    let _ = bus.write_u32(arg(0), h);
                }
                0
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

            // 0x00E2: NtPulseEvent(handle, previous_state_ptr)
            0x00E2 => match self.handle_object(arg(0), ObjectKind::Event) {
                Ok(ev) => {
                    let previous = objects::pulse_event(bus, ev);
                    if arg(1) != 0 {
                        let _ = bus.write_u32(arg(1), previous as u32);
                    }
                    0
                }
                Err(status) => status,
            },

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
                    println!("    [VFS] NtReadFile: read {} bytes from '{}' (pos: {}/{})", to_read, file.name, file.position, file.data.len());
                    if event_h != 0 {
                        if let Ok(ev) = self.handle_object(event_h as u64, ObjectKind::Event) {
                            objects::set_event(bus, ev);
                        }
                    }
                    0 // STATUS_SUCCESS
                } else {
                    0xC000_0008 // STATUS_INVALID_HANDLE
                }
            }

            // 0x00F3: NtReleaseSemaphore(handle, release_count, previous_count_ptr)
            0x00F3 => {
                let (status, previous) = match self.handle_object(arg(0), ObjectKind::Semaphore) {
                    Ok(sem) => match objects::release_semaphore(bus, sem, arg(1) as u32) {
                        (true, previous) => (0, previous),
                        (false, previous) => (objects::STATUS_SEMAPHORE_LIMIT_EXCEEDED, previous),
                    },
                    Err(status) => (status, 0),
                };
                if arg(2) != 0 {
                    let _ = bus.write_u32(arg(2), previous);
                }
                status
            }

            // 0x00F4: NtRemoveIoCompletion
            0x00F4 => 0x0000_0102, // STATUS_TIMEOUT

            // 0x00F6: NtSetEvent(handle, previous_state_ptr) (xeNtSetEvent)
            0x00F6 => match self.handle_object(arg(0), ObjectKind::Event) {
                Ok(ev) => {
                    let previous = objects::set_event(bus, ev);
                    if arg(1) != 0 {
                        let _ = bus.write_u32(arg(1), previous as u32);
                    }
                    0
                }
                Err(status) => status,
            },

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

            // 0x00FB: NtSignalAndWaitForSingleObjectEx(signal_handle, wait_handle, wait_mode, alertable, timeout_ptr)
            // (XObject::SignalAndWait): signals an event / semaphore (1) / mutant, then waits on the other object
            0x00FB => {
                let signal = self.objects.lookup(arg(0) as u32, self.current_thread).cloned();
                let target = self.objects.lookup(arg(1) as u32, self.current_thread).cloned();
                let (Some(signal), Some(target)) = (signal, target) else { return Some(objects::STATUS_INVALID_HANDLE) };
                let p = signal.guest_ptr as u64;
                match signal.kind {
                    ObjectKind::Event => {
                        objects::set_event(bus, p);
                    }
                    ObjectKind::Semaphore => {
                        objects::release_semaphore(bus, p, 1);
                    }
                    ObjectKind::Mutant => {
                        let _ = objects::release_mutant(bus, p, self.current_kthread);
                    }
                    _ => return Some(objects::STATUS_OBJECT_TYPE_MISMATCH),
                }
                self.wait_on(bus, target.guest_ptr, arg(4))
            }

            // 0x00FC: NtSuspendThread
            0x00FC => 0,

            // 0x00FD: NtWaitForSingleObjectEx(handle, wait_mode, alertable, timeout_ptr) - blocks on the scheduler
            0x00FD => match self.objects.lookup(arg(0) as u32, self.current_thread).map(|o| o.guest_ptr) {
                Some(object) => self.wait_on(bus, object, arg(3)),
                None => objects::STATUS_INVALID_HANDLE,
            },

            // 0x00FE: NtWaitForMultipleObjectsEx(count, handles, wait_type (0 all, 1 any), wait_mode, alertable,
            // timeout_ptr) - blocks on the scheduler. Objects without a dispatcher header here (files, I/O completion
            // ports) count as signaled.
            0x00FE => {
                let count = arg(0);
                if count == 0 || count > 64 || arg(2) > 1 {
                    return Some(objects::STATUS_INVALID_PARAMETER);
                }
                let mut headers = Vec::new();
                for i in 0..count {
                    let handle = bus.read_u32(arg(1) + 4 * i).unwrap_or(0);
                    match self.objects.lookup(handle, self.current_thread).map(|o| o.guest_ptr) {
                        None => return Some(objects::STATUS_INVALID_HANDLE),
                        Some(0) if arg(2) == 1 => return Some(scheduler::STATUS_WAIT_0 + i),
                        Some(0) => {}
                        Some(p) => headers.push(p),
                    }
                }
                if headers.is_empty() {
                    return Some(scheduler::STATUS_WAIT_0);
                }
                let deadline = self.wait_deadline(bus, arg(5));
                let wait = scheduler::WaitRequest::objects(self.current_kthread, headers, arg(2) == 0, deadline);
                self.begin_wait(bus, wait)
            }

            // 0x00FF: NtWriteFile
            0x00FF => 0,

            // 0x0101: NtYieldExecution
            0x0101 => 0,

            // 0x0103: ObCreateSymbolicLink | 0x0104: ObDeleteSymbolicLink
            0x0103 | 0x0104 => 0,

            // 0x0105: ObDereferenceObject(native_ptr): releases a reference taken by ObReferenceObject*
            0x0105 => {
                if let Some(h) = self.objects.handle_of_guest_ptr(arg(0) as u32) {
                    self.objects.release(h, 0);
                }
                0
            }

            // 0x010B: ObLookupThreadByThreadId(thread_id, out_object_ptr) -> the X_KTHREAD (handle retained)
            0x010B => {
                let thread = self.thread_by_id.get(&(arg(0) as u32)).copied();
                let kthread = thread.and_then(|h| self.objects.lookup(h, 0)).map(|o| o.guest_ptr).unwrap_or(0);
                if arg(1) != 0 {
                    let _ = bus.write_u32(arg(1), kthread);
                }
                match thread {
                    Some(h) if kthread != 0 => {
                        self.objects.retain(h, 0);
                        0
                    }
                    _ => objects::STATUS_INVALID_PARAMETER,
                }
            }

            // 0x010E: ObOpenObjectByPointer(object_ptr, out_handle_ptr): the handle of a known object, retained
            0x010E => {
                if arg(1) != 0 {
                    let _ = bus.write_u32(arg(1), 0);
                }
                match self.objects.handle_of_guest_ptr(arg(0) as u32) {
                    Some(h) => {
                        self.objects.retain(h, 0);
                        if arg(1) != 0 {
                            let _ = bus.write_u32(arg(1), h);
                        }
                        0
                    }
                    None => objects::STATUS_UNSUCCESSFUL,
                }
            }

            // 0x010F: ObReferenceObject(native_ptr): one more reference to a known object
            0x010F => {
                if let Some(h) = self.objects.handle_of_guest_ptr(arg(0) as u32) {
                    self.objects.retain(h, 0);
                }
                0
            }

            // 0x0110: ObReferenceObjectByHandle(handle, object_type_ptr, out_object_ptr) -> the guest object, with
            // the handle retained until ObDereferenceObject. The type check is skipped: the Ex*ObjectType kernel
            // variables are not exported to the guest yet, so the dash passes the raw import record.
            0x0110 => {
                if arg(2) != 0 {
                    let _ = bus.write_u32(arg(2), 0);
                }
                let handle = arg(0) as u32;
                match self.objects.lookup(handle, self.current_thread).map(|o| o.guest_ptr) {
                    Some(native) => {
                        self.objects.retain(handle, self.current_thread);
                        if arg(2) != 0 {
                            let _ = bus.write_u32(arg(2), native);
                        }
                        0
                    }
                    None => objects::STATUS_INVALID_HANDLE,
                }
            }

            _ => return None,
        })
    }

    /// Waits on one object for an Nt* wait: objects without a dispatcher header here (files, I/O completion ports)
    /// count as signaled.
    fn wait_on(&mut self, bus: &mut MemoryBus, object: u32, timeout_ptr: u64) -> u64 {
        if object == 0 {
            return scheduler::STATUS_WAIT_0;
        }
        let deadline = self.wait_deadline(bus, timeout_ptr);
        let wait = scheduler::WaitRequest::objects(self.current_kthread, vec![object], false, deadline);
        self.begin_wait(bus, wait)
    }
}
