//! xboxkrnl.exe HLE: Ke*/Kf*/Ki*: kernel scheduling primitives (events, waits, IRQL, DPC, timers, spinlocks).

use super::super::dispatcher::KernelDispatcher;
use crate::kernel::{objects, scheduler};
use crate::memory::MemoryBus;

/// X_KPCR::current_irql (Xenia xthread.h).
const KPCR_CURRENT_IRQL: u64 = 0x18;
/// DISPATCH_LEVEL.
const DISPATCH_LEVEL: u64 = 2;

impl KernelDispatcher {
    /// xeKfRaiseIrql: the caller's KPCR current_irql becomes `irql`; returns the previous one.
    fn raise_irql(&mut self, bus: &mut MemoryBus, irql: u64) -> u64 {
        let p = self.current_pcr as u64 + KPCR_CURRENT_IRQL;
        let old = bus.read_u8(p).unwrap_or(0) as u64;
        let _ = bus.write_u8(p, irql as u8);
        old
    }

    /// xeKeKfAcquireSpinLock: the lock word holds the owner's KPCR. A free lock is taken at once; a held one parks
    /// the caller until it is free (`WakeAction::AcquireSpinLock`). Returns `result` (the previous IRQL).
    fn acquire_spin_lock(&mut self, bus: &mut MemoryBus, lock: u64, result: u64) -> u64 {
        let owner = bus.read_u32(lock).unwrap_or(0);
        if owner == 0 {
            let _ = bus.write_u32(lock, self.current_pcr);
            return result;
        }
        if owner == self.current_pcr {
            println!("    [SPINLOCK] 0x{:08X} acquired again by its owner (KPCR 0x{:08X}): the thread never resumes", lock, owner);
        }
        self.pending_wait = Some(scheduler::WaitRequest {
            on_wake: scheduler::WakeAction::AcquireSpinLock { lock: lock as u32, pcr: self.current_pcr, result },
            ..scheduler::WaitRequest::objects(self.current_kthread, Vec::new(), false, None)
        });
        result
    }

    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_ke(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x004D: KeAcquireSpinLockAtRaisedIrql(lock): the lock without changing the IRQL; returns 0
            0x004D => self.acquire_spin_lock(bus, args.first().copied().unwrap_or(0), 0),

            // 0x0052: KeBugCheck | 0x0053: KeBugCheckEx
            0x0052 | 0x0053 => {
                println!("    [!] GUEST KeBugCheck called with code 0x{:X}", args.first().unwrap_or(&0));
                0
            }

            // 0x005A: KeDelayExecutionThread(processor_mode, alertable, interval_ptr) -> X_STATUS_SUCCESS after the
            // interval (XThread::Delay): the thread is parked until then (kernel/scheduler.rs); 0 only yields.
            // User APCs are not emulated, so alertable delays are not cut short.
            0x005A => {
                let interval_ptr = args.get(2).copied().unwrap_or(0);
                let interval = if interval_ptr != 0 { bus.read_u64(interval_ptr).unwrap_or(0) as i64 } else { 0 };
                if crate::diag::config().log_syscalls {
                    println!("    -> KeDelayExecutionThread: interval {} (100 ns units, negative = relative)", interval);
                }
                if let Some(deadline) = scheduler::deadline(bus.timebase.now(), interval) {
                    self.pending_wait = Some(scheduler::WaitRequest::delay(deadline));
                }
                0
            }

            // 0x005F: KeEnterCriticalRegion
            0x005F => 0,

            // 0x0066: KeGetCurrentProcessType
            // Returns 2 for System process (Dashboard), 1 for Title
            0x0066 => 2,

            // 0x006B: KeLockL2 | 0x006C: KeUnlockL2
            0x006B | 0x006C => 0,

            // 0x0075: KeInitializeTimerEx(timer, type, process_type) (xeKeInitializeTimerEx): type 0 notification,
            // 1 synchronization; not set, not signaled
            0x0075 => {
                let timer = args.first().copied().unwrap_or(0);
                if timer != 0 {
                    let kind = objects::TIMER_NOTIFICATION_OBJECT + (args.get(1).copied().unwrap_or(0) & 1) as u8;
                    let _ = bus.write_u8(timer, kind);
                    let _ = bus.write_u8(timer + 2, args.get(2).copied().unwrap_or(0) as u8); // process_type
                    let _ = bus.write_u8(timer + objects::HEADER_INSERTED, 0);
                    let _ = bus.write_u32(timer + 4, 0);
                    let _ = bus.write_u32(timer + 8, (timer + 8) as u32); // empty wait list
                    let _ = bus.write_u32(timer + 12, (timer + 8) as u32);
                    let _ = bus.write_u64(timer + objects::KTIMER_DUE_TIME, 0);
                    let _ = bus.write_u32(timer + objects::KTIMER_PERIOD, 0);
                    self.timers.cancel(timer as u32);
                }
                0
            }

            // 0x00A6: KeSetTimer(timer, due_time, dpc) | 0x00A7: KeSetTimerEx(timer, due_time, period_ms, dpc)
            // -> TRUE if the timer was already set (NT semantics; Xenia does not implement these). The due time is a
            // LARGE_INTEGER passed by value (negative = relative); the DPC runs when the timer expires.
            0x00A6 | 0x00A7 => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let (period_ms, dpc) = if ordinal == 0x00A6 { (0, arg(2)) } else { (arg(2), arg(3)) };
                self.set_timer(bus, arg(0), arg(1) as i64, period_ms as u32, dpc as u32)
            }

            // 0x0054: KeCancelTimer(timer) -> TRUE if it was set
            0x0054 => {
                let timer = args.first().copied().unwrap_or(0);
                if timer == 0 {
                    return Some(0);
                }
                let was_set = bus.read_u8(timer + objects::HEADER_INSERTED).unwrap_or(0) != 0;
                let _ = bus.write_u8(timer + objects::HEADER_INSERTED, 0);
                self.timers.cancel(timer as u32);
                was_set as u64
            }

            // 0x0070: KeInitializeEvent(event, event_type, initial_state): a KEVENT the guest owns; operated on in place
            0x0070 => {
                let p_event = args.first().copied().unwrap_or(0);
                if p_event != 0 {
                    let event_type = args.get(1).copied().unwrap_or(0) as u8;
                    let initial = (args.get(2).copied().unwrap_or(0) != 0) as u32;
                    objects::init_header(bus, p_event, event_type, objects::KEVENT_SIZE, initial);
                    // empty wait list
                    let _ = bus.write_u32(p_event + 8, (p_event + 8) as u32);
                    let _ = bus.write_u32(p_event + 12, (p_event + 8) as u32);
                }
                0
            }

            // 0x006F: KeInitializeDpc(dpc, routine, context) - XDPC layout as Xenia (kernel/dpc.rs)
            0x006F => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                super::super::dpc::initialize_dpc(bus, arg(0), arg(1), arg(2));
                0
            }

            // 0x007B: KeInsertQueueDpc(dpc, arg1, arg2) - the routine runs inline on the calling thread (frame loop)
            0x007B => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let (ret, call) = super::super::dpc::insert_queue_dpc(bus, &mut self.queued_dpcs, arg(0), arg(1), arg(2));
                self.pending_dpc_call = call;
                ret
            }

            // 0x008E: KeRemoveQueueDpc(dpc)
            0x008E => super::super::dpc::remove_queue_dpc(&mut self.queued_dpcs, args.first().copied().unwrap_or(0)),

            // 0x0074: KeInitializeSemaphore(semaphore, count, limit) (Xenia xboxkrnl_threading.cc)
            0x0074 => {
                let p_sem = args.first().copied().unwrap_or(0);
                if p_sem != 0 {
                    let count = args.get(1).copied().unwrap_or(0) as u32;
                    objects::init_header(bus, p_sem, objects::SEMAPHORE_OBJECT, objects::KSEMAPHORE_SIZE, count);
                    let _ = bus.write_u32(p_sem + objects::KSEMAPHORE_LIMIT, args.get(2).copied().unwrap_or(0) as u32);
                }
                0
            }

            // 0x007D: KeLeaveCriticalRegion
            0x007D => 0,

            // 0x007F: KePulseEvent(event, increment, wait) -> previous signal state (XEvent::Pulse)
            0x007F => match args.first().copied().unwrap_or(0) {
                0 => 0,
                p_event => objects::pulse_event(bus, p_event),
            },

            // 0x0081: KeQueryBasePriorityThread
            0x0081 => 8, // Normal thread priority

            // 0x0085: KeRaiseIrqlToDpcLevel() -> previous IRQL
            0x0085 => self.raise_irql(bus, DISPATCH_LEVEL),

            // 0x0088: KeReleaseSemaphore(semaphore, increment, adjustment, wait) -> previous count
            0x0088 => match args.first().copied().unwrap_or(0) {
                0 => 0,
                p_sem => objects::release_semaphore(bus, p_sem, args.get(2).copied().unwrap_or(0) as u32).1 as u64,
            },

            // 0x0083: KeQueryPerformanceFrequency() -> frequency. No arguments: returns 50 MHz
            // (Xenia Canary xboxkrnl_threading.cc: Clock::guest_tick_frequency()).
            0x0083 => self.host.time().query_performance_frequency(),

            // 0x0084: KeQuerySystemTime (pCurrentTime)
            0x0084 => {
                if let Some(&p_time) = args.first() {
                    if p_time != 0 {
                        let now = bus.timebase.now();
                        let ft = if now > 0 {
                            scheduler::system_time(now)
                        } else {
                            self.host.time().now_filetime()
                        };
                        let _ = bus.write_u64(p_time, ft);
                    }
                }
                0
            }

            // 0x0089: KeReleaseSpinLockFromRaisedIrql(lock): the lock word back to 0, the IRQL unchanged
            0x0089 => {
                let _ = bus.write_u32(args.first().copied().unwrap_or(0), 0);
                0
            }

            // 0x00AE: KeTryToAcquireSpinLockAtRaisedIrql(lock) -> 1 if taken, 0 if another owner holds it (one
            // compare-and-swap, no wait)
            0x00AE => {
                let lock = args.first().copied().unwrap_or(0);
                if bus.read_u32(lock).unwrap_or(0) == 0 {
                    let _ = bus.write_u32(lock, self.current_pcr);
                    1
                } else {
                    0
                }
            }

            // 0x008D: KeRemoveQueue
            0x008D => 0,

            // 0x008F: KeResetEvent(event) -> 1 (XEvent::Reset)
            0x008F => match args.first().copied().unwrap_or(0) {
                0 => 0,
                p_event => objects::reset_event(bus, p_event),
            },

            // 0x0092: KeResumeThread(kthread) -> previous suspend count
            0x0092 => match args.first().copied().unwrap_or(0) {
                0 => 0,
                kthread => objects::adjust_suspend_count(bus, kthread, -1) as u64,
            },

            // 0x02A4: KeGetImagePageTableEntry(address) (Xenia xboxkrnl_memory.cc): for the XEX heaps (0x80000000,
            // 64 KB pages; 0x90000000, 4 KB pages) the page index, with 0x40000001 for 4 KB pages (the 17559 dash
            // calls KeBugCheck(0xF7) when bit 0 is clear), masked with 0x400FFFFF; 0 elsewhere
            0x02A4 => {
                let address = args.first().copied().unwrap_or(0) as u32;
                let value = match address {
                    0x8000_0000..=0x8FFF_FFFF => (address - 0x8000_0000) / 0x1_0000,
                    0x9000_0000..=0x9FFF_FFFF => (address - 0x9000_0000) / 0x1000 | 0x4000_0001,
                    _ => 0,
                };
                (value & 0x400F_FFFF) as u64
            }

            // 0x0082: KeQueryInterruptTime() -> time since boot in 100 ns units (NT semantics; not in Xenia)
            0x0082 => bus.timebase.now() / 5,

            // 0x0097: KeSetAffinityThread
            0x0097 => 1,

            // 0x0099: KeSetBasePriorityThread
            0x0099 => 0,

            // 0x009D: KeSetEvent(event, increment, wait) -> 1 (XEvent::Set)
            0x009D => match args.first().copied().unwrap_or(0) {
                0 => 0,
                p_event => objects::set_event(bus, p_event),
            },

            // 0x00AF: KeWaitForMultipleObjects(count, objects_ptr, wait_type (0 all, 1 any), reason, mode, alertable,
            // timeout_ptr, wait_blocks) - blocks on the scheduler (kernel/scheduler.rs)
            0x00AF => {
                let arg = |i: usize| args.get(i).copied().unwrap_or(0);
                let count = arg(0).min(64);
                let objects: Vec<u32> = (0..count).map(|i| bus.read_u32(arg(1) + 4 * i).unwrap_or(0)).collect();
                if objects.is_empty() || objects.contains(&0) || arg(2) > 1 {
                    return Some(objects::STATUS_INVALID_PARAMETER);
                }
                let deadline = self.wait_deadline(bus, arg(6));
                let wait = scheduler::WaitRequest::objects(self.current_kthread, objects, arg(2) == 0, deadline);
                self.begin_wait(bus, wait)
            }

            // 0x00B0: KeWaitForSingleObject(object, reason, mode, alertable, timeout_ptr) - blocks on the scheduler
            0x00B0 => {
                let object = args.first().copied().unwrap_or(0) as u32;
                if object == 0 {
                    return Some(objects::STATUS_INVALID_PARAMETER);
                }
                let deadline = self.wait_deadline(bus, args.get(4).copied().unwrap_or(0));
                let wait = scheduler::WaitRequest::objects(self.current_kthread, vec![object], false, deadline);
                self.begin_wait(bus, wait)
            }

            // 0x00B1: KfAcquireSpinLock(lock) -> previous IRQL: raises to DISPATCH_LEVEL, then takes the lock
            0x00B1 => {
                let old = self.raise_irql(bus, DISPATCH_LEVEL);
                self.acquire_spin_lock(bus, args.first().copied().unwrap_or(0), old)
            }

            // 0x00B2: KfRaiseIrql(new_irql) -> previous IRQL
            0x00B2 => self.raise_irql(bus, args.first().copied().unwrap_or(0) & 0xFF),

            // 0x00B3: KfLowerIrql(new_irql)
            0x00B3 => {
                self.raise_irql(bus, args.first().copied().unwrap_or(0) & 0xFF);
                0
            }

            // 0x00B4: KfReleaseSpinLock(lock, old_irql): the lock word back to 0, then the IRQL restored
            0x00B4 => {
                let _ = bus.write_u32(args.first().copied().unwrap_or(0), 0);
                self.raise_irql(bus, args.get(1).copied().unwrap_or(0) & 0xFF);
                0
            }

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

#[cfg(test)]
mod tests {
    use crate::kernel::scheduler::{evaluate, WaitOutcome};
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn spin_locks_and_irql_follow_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        let (pcr_a, pcr_b, lock) = (0x9310_0000u32, 0x9310_2000u32, 0x4000_5A94u64);
        let call = |k: &mut KernelDispatcher, bus: &mut MemoryBus, pcr: u32, ordinal: u16, args: &[u64]| {
            k.current_pcr = pcr;
            k.pending_wait = None;
            k.dispatch("xboxkrnl.exe", ordinal, args, bus)
        };
        // KfAcquireSpinLock: IRQL 0 -> 2 (previous returned), the lock holds the owner's KPCR
        assert_eq!(call(&mut k, &mut bus, pcr_a, 0x00B1, &[lock]), 0);
        assert_eq!((bus.read_u32(lock).unwrap(), bus.read_u8(pcr_a as u64 + 0x18).unwrap()), (pcr_a, 2));
        assert!(k.pending_wait.is_none());
        // another thread: parked; its IRQL is already raised and the previous one is its result
        assert_eq!(call(&mut k, &mut bus, pcr_b, 0x00B1, &[lock]), 0);
        let wait = k.pending_wait.take().expect("parked on the held lock");
        assert_eq!(evaluate(&mut bus, &wait, u64::MAX), WaitOutcome::Pending);
        // KeTryToAcquireSpinLockAtRaisedIrql fails while it is held
        assert_eq!(call(&mut k, &mut bus, pcr_b, 0x00AE, &[lock]), 0);
        // KfReleaseSpinLock(lock, 0): free, IRQL restored; the parked thread then takes it
        assert_eq!(call(&mut k, &mut bus, pcr_a, 0x00B4, &[lock, 0]), 0);
        assert_eq!((bus.read_u32(lock).unwrap(), bus.read_u8(pcr_a as u64 + 0x18).unwrap()), (0, 0));
        assert_eq!(evaluate(&mut bus, &wait, 0), WaitOutcome::Done(0));
        assert_eq!(bus.read_u32(lock).unwrap(), pcr_b);
        // the ...AtRaisedIrql pair leaves the IRQL alone
        call(&mut k, &mut bus, pcr_b, 0x0089, &[lock]);
        bus.write_u8(pcr_a as u64 + 0x18, 1).unwrap();
        assert_eq!(call(&mut k, &mut bus, pcr_a, 0x004D, &[lock]), 0);
        assert_eq!((bus.read_u32(lock).unwrap(), bus.read_u8(pcr_a as u64 + 0x18).unwrap()), (pcr_a, 1));
        call(&mut k, &mut bus, pcr_a, 0x0089, &[lock]);
        assert_eq!(call(&mut k, &mut bus, pcr_b, 0x00AE, &[lock]), 1);
        assert_eq!(bus.read_u32(lock).unwrap(), pcr_b);
        // KfRaiseIrql / KfLowerIrql / KeRaiseIrqlToDpcLevel
        assert_eq!(call(&mut k, &mut bus, pcr_a, 0x00B2, &[4]), 1);
        assert_eq!(call(&mut k, &mut bus, pcr_a, 0x00B3, &[0]), 0);
        assert_eq!(call(&mut k, &mut bus, pcr_a, 0x0085, &[]), 0);
        assert_eq!(bus.read_u8(pcr_a as u64 + 0x18).unwrap(), 2);
    }

    #[test]
    fn query_performance_frequency_returns_50mhz_and_writes_nothing() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // the dash's CP-interrupt handler calls it with r3 = 0x100 (leftover argument)
        bus.write_u64(0x100, 0x1122_3344_5566_7788).unwrap();
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0083, &[0x100], &mut bus), 50_000_000);
        assert_eq!(bus.read_u64(0x100).unwrap(), 0x1122_3344_5566_7788);
        // the handler divides freq by the refresh rate before a divide-by-zero guard
        assert_ne!(50_000_000u64 / 60, 0);
    }

    #[test]
    fn image_page_table_entry_follows_xenia() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        let mut pte = |a: u64| k.dispatch("xboxkrnl.exe", 0x02A4, &[a], &mut bus);
        // dash code at 0x928EF464 (4 KB XEX heap): page 0x28EF, flags 0x40000001
        assert_eq!(pte(0x928E_F464), 0x4000_28EF);
        assert_eq!(pte(0x9000_0000), 0x4000_0001);
        assert_eq!(pte(0x8001_0000), 1, "64 KB XEX heap: page index only");
        assert_eq!(pte(0x4000_0000), 0, "not a XEX heap");
    }
}
