//! xboxkrnl.exe HLE: Ke*/Kf*/Ki*: kernel scheduling primitives (events, waits, IRQL, DPC, timers, spinlocks).

use super::super::dispatcher::{KernelDispatcher, KernelEvent};
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_ke(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x004D: KeAcquireSpinLockAtRaisedIrql
            0x004D => 0,

            // 0x0052: KeBugCheck | 0x0053: KeBugCheckEx
            0x0052 | 0x0053 => {
                println!("    [!] GUEST KeBugCheck called with code 0x{:X}", args.first().unwrap_or(&0));
                0
            }

            // 0x0057: KeDelayExecutionThread (waitMode, alertable, pInterval)
            0x0057 => 0,

            // 0x005F: KeEnterCriticalRegion
            0x005F => 0,

            // 0x0066: KeGetCurrentProcessType
            // Returns 2 for System process (Dashboard), 1 for Title
            0x0066 => 2,

            // 0x006B: KeLockL2 | 0x006C: KeUnlockL2
            0x006B | 0x006C => 0,

            // 0x0070: KeInitializeEvent (pEvent, eventType, initialState)
            0x0070 => {
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        let event_type = args.get(1).copied().unwrap_or(0) as u32;
                        let initial = args.get(2).copied().unwrap_or(0) as u32;

                        // KEVENT memory layout:
                        // 0x00: Header.Type (0 = Notification, 1 = Synchronization)
                        // 0x01: Header.Absolute
                        // 0x02: Header.Size (4 dwords = 16 bytes)
                        // 0x03: Header.Inserted
                        // 0x04: Header.SignalState
                        // 0x08: WaitListHead.Flink
                        // 0x0C: WaitListHead.Blink
                        let _ = bus.write_u8(p_event, event_type as u8);
                        let _ = bus.write_u8(p_event + 1, 0);
                        let _ = bus.write_u8(p_event + 2, 4);
                        let _ = bus.write_u8(p_event + 3, 0);
                        let _ = bus.write_u32(p_event + 4, initial);
                        let _ = bus.write_u32(p_event + 8, (p_event + 8) as u32);
                        let _ = bus.write_u32(p_event + 12, (p_event + 8) as u32);

                        let h = self.next_handle;
                        self.next_handle += 4;
                        self.events.insert(h, KernelEvent {
                            handle: h,
                            event_type,
                            signaled: initial != 0,
                            guest_ptr: p_event,
                        });
                        self.event_by_ptr.insert(p_event, h);
                    }
                }
                0
            }

            // 0x006F: KeInitializeDpc (pDpc, pRoutine, pContext)
            0x006F => {
                if let (Some(&p_dpc), Some(&p_routine), Some(&p_context)) = (args.first(), args.get(1), args.get(2)) {
                    if p_dpc != 0 {
                        let _ = bus.write_u32(p_dpc + 8, p_routine as u32);
                        let _ = bus.write_u32(p_dpc + 12, p_context as u32);
                    }
                }
                0
            }

            // 0x0074: KeInitializeSemaphore (pSemaphore, count, limit)
            0x0074 => {
                if let Some(&p_sem) = args.first() {
                    if p_sem != 0 {
                        for off in (0..20).step_by(4) {
                            let _ = bus.write_u32(p_sem + off, 0);
                        }
                        let count = args.get(1).copied().unwrap_or(1) as u32;
                        let _ = bus.write_u32(p_sem + 4, count);
                    }
                }
                0
            }

            // 0x007D: KeLeaveCriticalRegion
            0x007D => 0,

            // 0x007F: KePulseEvent (pEvent, increment, wait)
            0x007F => {
                let mut prev = 0u64;
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        if let Ok(st) = bus.read_u32(p_event + 4) {
                            prev = st as u64;
                        }
                        let _ = bus.write_u32(p_event + 4, 0);
                        if let Some(&h) = self.event_by_ptr.get(&p_event) {
                            if let Some(ev) = self.events.get_mut(&h) {
                                ev.signaled = false;
                            }
                        }
                    }
                }
                prev
            }

            // 0x0080: KeQueryActiveProcessors
            0x0080 => 0x3F, // 6 hardware threads (3 cores x 2 SMT)

            // 0x0081: KeQueryBasePriorityThread
            0x0081 => 8, // Normal thread priority

            // 0x0085: KeRaiseIrqlToDpcLevel
            0x0085 => 2, // Previous IRQL (DISPATCH_LEVEL)

            // 0x0088: KeReleaseSemaphore
            0x0088 => 1,

            // 0x0083: KeQueryPerformanceFrequency (pFrequency)
            0x0083 => {
                if let Some(&p_freq) = args.first() {
                    if p_freq != 0 {
                        let _ = bus.write_u64(p_freq, 50_000_000);
                    }
                }
                1 // TRUE
            }

            // 0x0084: KeQuerySystemTime (pCurrentTime)
            0x0084 => {
                if let Some(&p_time) = args.first() {
                    if p_time != 0 {
                        let ft = 133_000_000_000_000_000u64 + (self.next_handle as u64 * 10_000_000);
                        let _ = bus.write_u64(p_time, ft);
                    }
                }
                0
            }

            // 0x0089: KeReleaseSpinLockFromRaisedIrql
            0x0089 => 0,

            // 0x008D: KeRemoveQueue | 0x0090: KeRundownQueue | 0x0095: KeInsertQueue | 0x0096: KeInsertHeadQueue | 0x0098: KeSetCurrentStackPointers
            0x008D | 0x0090 | 0x0095 | 0x0096 | 0x0098 => 0,

            // 0x008F: KeResetEvent
            0x008F => {
                let mut prev = 0u64;
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        if let Ok(st) = bus.read_u32(p_event + 4) {
                            prev = st as u64;
                        }
                        let _ = bus.write_u32(p_event + 4, 0);
                        if let Some(&h) = self.event_by_ptr.get(&p_event) {
                            if let Some(ev) = self.events.get_mut(&h) {
                                ev.signaled = false;
                            }
                        }
                    }
                }
                prev
            }

            // 0x0092: KeResumeThread
            0x0092 => 1, // Previous suspend count

            // 0x0097: KeSetAffinityThread
            0x0097 => 1,

            // 0x0099: KeSetBasePriorityThread
            0x0099 => 0,

            // 0x009D: KeSetEvent
            0x009D => {
                let mut prev = 0u64;
                if let Some(&p_event) = args.first() {
                    if p_event != 0 {
                        if let Ok(st) = bus.read_u32(p_event + 4) {
                            prev = st as u64;
                        }
                        let _ = bus.write_u32(p_event + 4, 1);
                        if let Some(&h) = self.event_by_ptr.get(&p_event) {
                            if let Some(ev) = self.events.get_mut(&h) {
                                ev.signaled = true;
                            }
                        }
                    }
                }
                prev
            }

            // 0x00AF: KeWaitForMultipleObjects
            0x00AF => {
                let count = args.first().copied().unwrap_or(1) as usize;
                let p_objects = args.get(1).copied().unwrap_or(0);
                if p_objects != 0 {
                    for i in 0..count.min(8) {
                        if let Ok(p_obj) = bus.read_u32(p_objects + (i * 4) as u64) {
                            if p_obj != 0 {
                                if let Ok(typ) = bus.read_u8(p_obj as u64) {
                                    if typ == 1 {
                                        let _ = bus.write_u32(p_obj as u64 + 4, 0);
                                    }
                                }
                            }
                        }
                    }
                }
                0 // STATUS_WAIT_0 / STATUS_SUCCESS
            }

            // 0x00B0: KeWaitForSingleObject
            0x00B0 => {
                if let Some(&p_obj) = args.first() {
                    if p_obj != 0 {
                        let is_sync = bus.read_u8(p_obj).unwrap_or(0) == 1;
                        if is_sync {
                            let _ = bus.write_u32(p_obj + 4, 0);
                        }
                    }
                }
                0 // STATUS_SUCCESS / WAIT_OBJECT_0
            }

            // 0x00B1: KfAcquireSpinLock | 0x00B2: KfRaiseIrql
            0x00B1 | 0x00B2 => 0,

            // 0x00B3: KfLowerIrql
            0x00B3 => 0,

            // 0x00B4: KfReleaseSpinLock
            0x00B4 => 0,

            // 0x0152: KeTlsAlloc
            0x0152 => {
                let slot = self.next_tls_index;
                self.next_tls_index += 1;
                slot as u64
            }

            // 0x0153: KeTlsFree
            0x0153 => 1, // TRUE

            // 0x0154: KeTlsGetValue (slot)
            0x0154 => {
                let slot = args.first().copied().unwrap_or(0) as u32;
                self.tls_slots.get(&slot).copied().unwrap_or(0)
            }

            // 0x0155: KeTlsSetValue (slot, value)
            0x0155 => {
                let slot = args.first().copied().unwrap_or(0) as u32;
                let val = args.get(1).copied().unwrap_or(0);
                if slot == 1 {
                    println!("    [TLS SET] slot=1, val=0x{:X}", val);
                }
                self.tls_slots.insert(slot, val);
                1 // TRUE
            }

            // 0x01DF: KiApcNormalRoutineNop
            0x01DF => 0,

            // 0x0266: KeCertMonitorData
            0x0266 => 0,

            _ => return None,
        })
    }
}
