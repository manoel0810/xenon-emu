//! xboxkrnl.exe HLE: Ex*: executive (threads, pools, XConfig, read/write locks).

use super::super::dispatcher::{KernelDispatcher, PendingThread};
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_ex(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x000D: ExCreateThread
            // (pHandle, stackSize, pThreadId, apiStartup, startRoutine, startContext, creationFlags)
            0x000D => {
                let h_thread = self.next_handle;
                self.next_handle += 4;
                let tid = self.next_thread_id;
                self.next_thread_id += 1;

                if let Some(&p_handle) = args.first() {
                    if p_handle != 0 {
                        let _ = bus.write_u32(p_handle, h_thread);
                    }
                }
                if let Some(&p_tid) = args.get(2) {
                    if p_tid != 0 {
                        let _ = bus.write_u32(p_tid, tid);
                    }
                }

                let start_routine = args.get(4).copied().unwrap_or(0);
                let start_context = args.get(5).copied().unwrap_or(0);
                let stack_size = args.get(1).copied().unwrap_or(0x10000).max(0x10000) as u32;
                let creation_flags = args.get(6).copied().unwrap_or(0) as u32;

                println!("    -> ExCreateThread: Created thread TID=0x{:X}, Handle=0x{:X}, Entry=0x{:08X}, Param=0x{:08X}, Stack=0x{:X}",
                    tid, h_thread, start_routine, start_context, stack_size
                );

                if start_routine != 0 {
                    self.pending_threads.push(PendingThread {
                        tid,
                        handle: h_thread,
                        start_routine,
                        start_context,
                        stack_size,
                        creation_flags,
                    });
                }

                0 // STATUS_SUCCESS
            }

            // 0x0007: ExAcquireReadWriteLockExclusive | 0x0008: ExAcquireReadWriteLockShared
            0x0007 | 0x0008 => 0,

            // 0x000F: ExFreePool
            0x000F => 0,

            // 0x0010: ExGetXConfigSetting
            // (category, setting, pBuffer, size, pRequiredSize)
            0x0010 => {
                if let Some(&p_req) = args.get(4) {
                    if p_req != 0 {
                        let _ = bus.write_u16(p_req, 4);
                    }
                }
                if let (Some(&p_buf), Some(&sz)) = (args.get(2), args.get(3)) {
                    if p_buf != 0 && sz >= 4 {
                        let _ = bus.write_u32(p_buf, 1);
                    }
                }
                0 // STATUS_SUCCESS
            }

            // 0x0011: ExInitializeReadWriteLock
            0x0011 => {
                if let Some(&p_lock) = args.first() {
                    if p_lock != 0 {
                        for off in (0..16).step_by(4) {
                            let _ = bus.write_u32(p_lock + off, 0);
                        }
                    }
                }
                0
            }

            // 0x0016: ExReleaseReadWriteLock
            0x0016 => 0,

            // 0x0017: ExRegisterTitleTerminateNotification | 0x0018: ExUnregisterTitleTerminateNotification
            0x0017 | 0x0018 => 0,

            // 0x0019: ExTerminateThread
            0x0019 => 0,

            _ => return None,
        })
    }
}
