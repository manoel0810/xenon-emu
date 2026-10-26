// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xthread.cc                 XThread::Delay (interval units and signs)
//   src/xenia/kernel/xboxkrnl/xboxkrnl_threading.cc KeDelayExecutionThread, KeWaitFor*/NtWaitFor* statuses
//   src/xenia/xbox.h                            X_STATUS_TIMEOUT, STATUS_WAIT_0
//
//! Thread waits. Xenia blocks a host thread; here a guest thread that waits is parked by the frame loop with a
//! `WaitRequest` and runs no instructions until the request is satisfied or its deadline (on the global time base,
//! cpu/timebase.rs) passes. A thread is ready (no request), waiting (a request) or terminated (inactive).

use super::guest_mem::GuestMem;
use super::objects::{
    take_critical_section, EVENT_SYNCHRONIZATION_OBJECT, HEADER_SIGNAL_STATE, HEADER_TYPE, KMUTANT_OWNER, MUTANT_OBJECT,
    SEMAPHORE_OBJECT, TIMER_SYNCHRONIZATION_OBJECT,
};
use crate::cpu::timebase::TICKS_PER_SECOND;

pub const STATUS_WAIT_0: u64 = 0;
pub const STATUS_TIMEOUT: u64 = 0x102;

/// System time (FILETIME, 100 ns since 1601) at time base 0; KeQuerySystemTime is this plus the time base.
pub const SYSTEM_TIME_AT_BOOT: u64 = 133_000_000_000_000_000;
/// Time base ticks per 100 ns unit.
const TICKS_PER_100NS: u64 = TICKS_PER_SECOND / 10_000_000;

/// KeQuerySystemTime for a time base value.
pub fn system_time(timebase: u64) -> u64 {
    SYSTEM_TIME_AT_BOOT + timebase / TICKS_PER_100NS
}

/// Deadline (time base) of a kernel timeout `interval` (LARGE_INTEGER, 100 ns units): negative = relative to `now`,
/// positive = absolute system time (Xenia leaves this case as a TODO and does not wait; here it is converted with
/// `system_time`), 0 = no wait.
pub fn deadline(now: u64, interval: i64) -> Option<u64> {
    if interval < 0 {
        Some(now.saturating_add(interval.unsigned_abs().saturating_mul(TICKS_PER_100NS)))
    } else if interval > 0 {
        let absolute = (interval as u64).saturating_sub(SYSTEM_TIME_AT_BOOT).saturating_mul(TICKS_PER_100NS);
        Some(absolute.max(now))
    } else {
        None
    }
}

/// What a thread does once its wait is satisfied, before it resumes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeAction {
    None,
    /// RtlEnterCriticalSection: take ownership of the section at this address.
    EnterCriticalSection(u32),
}

/// A parked thread's wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaitRequest {
    /// The waiting thread's X_KTHREAD (mutant ownership, critical sections).
    pub thread: u32,
    pub on_wake: WakeAction,
    /// Dispatcher headers (guest addresses) waited on; empty for a plain delay.
    pub objects: Vec<u32>,
    /// All objects must be signaled (WaitAll) instead of any one (WaitAny).
    pub wait_all: bool,
    /// Time base value at which the wait gives up; None = no timeout.
    pub deadline: Option<u64>,
    /// Status returned when the deadline passes (STATUS_TIMEOUT for waits, STATUS_SUCCESS for a delay).
    pub timeout_status: u64,
}

impl WaitRequest {
    /// KeDelayExecutionThread: no objects, completes with STATUS_SUCCESS at the deadline.
    pub fn delay(deadline: u64) -> Self {
        Self { thread: 0, on_wake: WakeAction::None, objects: Vec::new(), wait_all: false, deadline: Some(deadline), timeout_status: 0 }
    }

    /// A wait on dispatcher objects, finishing with STATUS_TIMEOUT at `deadline`.
    pub fn objects(thread: u32, objects: Vec<u32>, wait_all: bool, deadline: Option<u64>) -> Self {
        Self { thread, on_wake: WakeAction::None, objects, wait_all, deadline, timeout_status: STATUS_TIMEOUT }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The thread resumes with this status in r3.
    Done(u64),
    Pending,
}

fn signaled(mem: &mut dyn GuestMem, header: u32, thread: u32) -> bool {
    let h = header as u64;
    if mem.read_u8(h + HEADER_TYPE) == MUTANT_OBJECT && thread != 0 && mem.read_u32(h + KMUTANT_OWNER) == thread {
        return true; // the owner acquires its mutant again
    }
    (mem.read_u32(h + HEADER_SIGNAL_STATE) as i32) > 0
}

/// Consumes what a satisfied wait takes from an object: a synchronization event or timer resets, a semaphore count
/// drops, a mutant gets (one more level of) `thread` as owner.
fn acquire(mem: &mut dyn GuestMem, header: u32, thread: u32) {
    let h = header as u64;
    let object_type = mem.read_u8(h + HEADER_TYPE);
    match object_type {
        EVENT_SYNCHRONIZATION_OBJECT | TIMER_SYNCHRONIZATION_OBJECT => mem.write_u32(h + HEADER_SIGNAL_STATE, 0),
        SEMAPHORE_OBJECT | MUTANT_OBJECT => {
            let count = mem.read_u32(h + HEADER_SIGNAL_STATE);
            mem.write_u32(h + HEADER_SIGNAL_STATE, count.wrapping_sub(1));
            if object_type == MUTANT_OBJECT {
                mem.write_u32(h + KMUTANT_OWNER, thread);
            }
        }
        _ => {}
    }
}

fn wake(mem: &mut dyn GuestMem, wait: &WaitRequest) {
    if let WakeAction::EnterCriticalSection(cs) = wait.on_wake {
        take_critical_section(mem, cs as u64, wait.thread);
    }
}

/// Checks a wait at time `now`: satisfied objects are acquired and STATUS_WAIT_0 (+ index for WaitAny) is returned;
/// otherwise the timeout status once the deadline has passed; otherwise Pending.
pub fn evaluate(mem: &mut dyn GuestMem, wait: &WaitRequest, now: u64) -> WaitOutcome {
    if !wait.objects.is_empty() {
        if wait.wait_all {
            if wait.objects.iter().all(|&o| signaled(mem, o, wait.thread)) {
                for &o in &wait.objects {
                    acquire(mem, o, wait.thread);
                }
                wake(mem, wait);
                return WaitOutcome::Done(STATUS_WAIT_0);
            }
        } else if let Some(i) = wait.objects.iter().position(|&o| signaled(mem, o, wait.thread)) {
            acquire(mem, wait.objects[i], wait.thread);
            wake(mem, wait);
            return WaitOutcome::Done(STATUS_WAIT_0 + i as u64);
        }
    }
    match wait.deadline {
        Some(d) if now >= d => WaitOutcome::Done(wait.timeout_status),
        _ => WaitOutcome::Pending,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;
    use crate::kernel::objects::{init_header, EVENT_NOTIFICATION_OBJECT, KEVENT_SIZE, KSEMAPHORE_SIZE};

    #[test]
    fn intervals_become_time_base_deadlines() {
        // -10 ms relative = 100 000 units of 100 ns = 500 000 ticks at 50 MHz
        assert_eq!(deadline(1_000, -100_000), Some(501_000));
        assert_eq!(deadline(1_000, 0), None);
        // absolute: the boot system time + 1 s
        assert_eq!(deadline(0, (SYSTEM_TIME_AT_BOOT + 10_000_000) as i64), Some(50_000_000));
        // absolute in the past: due now
        assert_eq!(deadline(7_000, (SYSTEM_TIME_AT_BOOT + 1) as i64), Some(7_000));
        assert_eq!(system_time(50_000_000), SYSTEM_TIME_AT_BOOT + 10_000_000);
    }

    #[test]
    fn a_delay_completes_at_its_deadline() {
        let mut m = FakeMem::default();
        let w = WaitRequest::delay(500);
        assert_eq!(evaluate(&mut m, &w, 499), WaitOutcome::Pending);
        assert_eq!(evaluate(&mut m, &w, 500), WaitOutcome::Done(0));
    }

    #[test]
    fn waits_on_objects_acquire_them() {
        let mut m = FakeMem::default();
        init_header(&mut m, 0x100, EVENT_SYNCHRONIZATION_OBJECT, KEVENT_SIZE, 0);
        init_header(&mut m, 0x200, EVENT_NOTIFICATION_OBJECT, KEVENT_SIZE, 0);
        init_header(&mut m, 0x300, SEMAPHORE_OBJECT, KSEMAPHORE_SIZE, 0);
        let any = WaitRequest::objects(0, vec![0x100, 0x200, 0x300], false, Some(1000));
        assert_eq!(evaluate(&mut m, &any, 10), WaitOutcome::Pending);
        // the semaphore gets a count: WaitAny returns its index and takes the count
        m.write_u32(0x304, 2);
        assert_eq!(evaluate(&mut m, &any, 10), WaitOutcome::Done(2));
        assert_eq!(m.read_u32(0x304), 1);
        // a synchronization event resets when it satisfies a wait; a notification event stays signaled
        m.write_u32(0x104, 1);
        m.write_u32(0x204, 1);
        let all = WaitRequest::objects(0, vec![0x100, 0x200], true, None);
        assert_eq!(evaluate(&mut m, &all, 10), WaitOutcome::Done(0));
        assert_eq!((m.read_u32(0x104), m.read_u32(0x204)), (0, 1));
        // WaitAll with one object not signaled never completes without a deadline
        assert_eq!(evaluate(&mut m, &all, u64::MAX), WaitOutcome::Pending);
        // timeout
        m.write_u32(0x304, 0);
        let none = WaitRequest::objects(0, vec![0x300], false, Some(1000));
        assert_eq!(evaluate(&mut m, &none, 1000), WaitOutcome::Done(STATUS_TIMEOUT));
    }

    #[test]
    fn ke_delay_execution_thread_parks_the_caller() {
        let mut k = crate::kernel::KernelDispatcher::new();
        let mut bus = crate::memory::MemoryBus::new();
        bus.timebase.begin_frame(10, 25_000); // now = 10 frames
        let now = bus.timebase.now();
        // 1 ms relative: -10 000 units of 100 ns
        bus.write_u64(0x4000_0000, (-10_000i64) as u64).unwrap();
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x005A, &[1, 0, 0x4000_0000], &mut bus), 0);
        assert_eq!(k.pending_wait, Some(WaitRequest::delay(now + 50_000)));
        // a zero interval only yields
        k.pending_wait = None;
        bus.write_u64(0x4000_0000, 0).unwrap();
        k.dispatch("xboxkrnl.exe", 0x005A, &[1, 0, 0x4000_0000], &mut bus);
        assert_eq!(k.pending_wait, None);
    }
}
