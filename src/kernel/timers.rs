// Partly derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia
// Canary. See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xthread.h                  X_KTIMER layout
//   src/xenia/kernel/xtimer.cc                  XTimer::SetTimer (due time sign, period in ms)
//   src/xenia/kernel/xiocompletion.h            IONotification
// KeSetTimer/KeSetTimerEx/KeCancelTimer are not implemented by Xenia; they follow the NT kernel semantics (a timer
// is set with a due time, an optional period and an optional DPC that runs when it expires).
//
//! Kernel timers on the global time base. A set timer has a deadline; when the frame loop reaches it the timer's
//! header is signaled (waiters wake through the scheduler), its DPC is returned to be run, and a periodic timer is
//! set again.

use std::collections::HashMap;

use super::dispatcher::KernelDispatcher;
use super::dpc::{DpcCall, XDPC_CONTEXT, XDPC_ROUTINE};
use super::guest_mem::GuestMem;
use super::objects::{HEADER_INSERTED, HEADER_SIGNAL_STATE, KTIMER_DPC, KTIMER_DUE_TIME, KTIMER_PERIOD};
use super::scheduler::{deadline, system_time};
use crate::cpu::timebase::TICKS_PER_SECOND;
use crate::memory::MemoryBus;

const TICKS_PER_MS: u64 = TICKS_PER_SECOND / 1000;

/// XIOCompletion::IONotification.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoNotification {
    pub key_context: u32,
    pub apc_context: u32,
    pub num_bytes: u32,
    pub status: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Timer {
    deadline: u64,
    period_ticks: u64,
    dpc: u32,
}

#[derive(Debug, Default)]
pub struct TimerQueue {
    timers: HashMap<u32, Timer>,
}

impl TimerQueue {
    pub fn cancel(&mut self, timer: u32) {
        self.timers.remove(&timer);
    }

    pub fn is_set(&self, timer: u32) -> bool {
        self.timers.contains_key(&timer)
    }

    /// Expires every timer due at `now`: signals it, re-arms periodic ones, and returns the DPCs to run with the
    /// system time (low, high) as arguments.
    pub fn fire(&mut self, mem: &mut dyn GuestMem, now: u64) -> Vec<DpcCall> {
        let mut due: Vec<(u32, Timer)> = self.timers.iter().filter(|(_, t)| t.deadline <= now).map(|(&k, &t)| (k, t)).collect();
        due.sort_by_key(|&(k, t)| (t.deadline, k));
        let mut calls = Vec::new();
        for (ptr, t) in due {
            let p = ptr as u64;
            mem.write_u32(p + HEADER_SIGNAL_STATE, 1);
            if t.period_ticks > 0 {
                let mut next = t.deadline + t.period_ticks;
                while next <= now {
                    next += t.period_ticks;
                }
                self.timers.insert(ptr, Timer { deadline: next, ..t });
            } else {
                mem.write_u8(p + HEADER_INSERTED, 0);
                self.timers.remove(&ptr);
            }
            if t.dpc != 0 {
                let routine = mem.read_u32(t.dpc as u64 + XDPC_ROUTINE);
                if routine != 0 {
                    let time = system_time(now);
                    calls.push(DpcCall { dpc: t.dpc, routine, context: mem.read_u32(t.dpc as u64 + XDPC_CONTEXT), arg1: time as u32, arg2: (time >> 32) as u32 });
                }
            }
        }
        calls
    }
}

impl KernelDispatcher {
    /// KeSetTimer(Ex) / NtSetTimerEx on a KTIMER: not signaled, set to expire at `due_time` (negative = relative,
    /// 100 ns units; 0 = now), then every `period_ms`. Returns TRUE if it was already set.
    pub fn set_timer(&mut self, bus: &mut MemoryBus, timer: u64, due_time: i64, period_ms: u32, dpc: u32) -> u64 {
        if timer == 0 {
            return 0;
        }
        let now = bus.timebase.now();
        let was_set = bus.read_u8(timer + HEADER_INSERTED).unwrap_or(0) != 0;
        let when = deadline(now, due_time).unwrap_or(now);
        let _ = bus.write_u32(timer + HEADER_SIGNAL_STATE, 0);
        let _ = bus.write_u8(timer + HEADER_INSERTED, 1);
        let _ = bus.write_u64(timer + KTIMER_DUE_TIME, system_time(when));
        let _ = bus.write_u32(timer + KTIMER_DPC, dpc);
        let _ = bus.write_u32(timer + KTIMER_PERIOD, period_ms);
        self.timers.timers.insert(timer as u32, Timer { deadline: when, period_ticks: period_ms as u64 * TICKS_PER_MS, dpc });
        was_set as u64
    }

    /// Expires the timers due now (see `TimerQueue::fire`).
    pub fn fire_timers(&mut self, bus: &mut MemoryBus) -> Vec<DpcCall> {
        let now = bus.timebase.now();
        self.timers.fire(bus, now)
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel::scheduler::{evaluate, WaitOutcome, WaitRequest};
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    fn call(k: &mut KernelDispatcher, bus: &mut MemoryBus, ordinal: u16, args: &[u64]) -> u64 {
        k.dispatch("xboxkrnl.exe", ordinal, args, bus)
    }

    #[test]
    fn ke_timer_expires_signals_and_queues_its_dpc() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        const T: u64 = 0x4100_0000;
        const DPC: u64 = 0x4100_0100;
        call(&mut k, &mut bus, 0x0075, &[T, 1, 0]); // synchronization timer
        call(&mut k, &mut bus, 0x006F, &[DPC, 0x9220_0000, 0x1234]); // KeInitializeDpc
        bus.timebase.begin_frame(1, 1);
        let now = bus.timebase.now();
        // KeSetTimerEx(timer, -1 ms, period 0, dpc)
        assert_eq!(call(&mut k, &mut bus, 0x00A7, &[T, (-10_000i64) as u64, 0, DPC]), 0);
        assert_eq!((bus.read_u8(T + 3).unwrap(), bus.read_u32(T + 0x20).unwrap()), (1, DPC as u32));
        let waiter = WaitRequest::objects(0x4200_0000, vec![T as u32], false, None);
        assert_eq!(evaluate(&mut bus, &waiter, now), WaitOutcome::Pending);
        assert!(k.timers.fire(&mut bus, now + 49_999).is_empty());
        let calls = k.timers.fire(&mut bus, now + 50_000);
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].routine, calls[0].context, calls[0].dpc), (0x9220_0000, 0x1234, DPC as u32));
        assert_eq!((bus.read_u32(T + 4).unwrap(), bus.read_u8(T + 3).unwrap()), (1, 0));
        // the synchronization timer wakes one waiter and resets
        assert_eq!(evaluate(&mut bus, &waiter, now + 50_000), WaitOutcome::Done(0));
        assert_eq!(bus.read_u32(T + 4).unwrap(), 0);
        // KeCancelTimer on an expired timer: FALSE
        assert_eq!(call(&mut k, &mut bus, 0x0054, &[T]), 0);
    }

    #[test]
    fn periodic_timers_rearm_and_cancel() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        const T: u64 = 0x4100_0000;
        call(&mut k, &mut bus, 0x0075, &[T, 0, 0]); // notification timer
        call(&mut k, &mut bus, 0x00A7, &[T, (-10_000i64) as u64, 2, 0]); // every 2 ms
        assert_eq!(call(&mut k, &mut bus, 0x00A7, &[T, (-10_000i64) as u64, 2, 0]), 1, "already set");
        k.timers.fire(&mut bus, 50_000);
        assert!(k.timers.is_set(T as u32), "periodic timers stay set");
        assert_eq!(bus.read_u32(T + 4).unwrap(), 1);
        assert_eq!(call(&mut k, &mut bus, 0x0054, &[T]), 1);
        assert!(!k.timers.is_set(T as u32));
    }

    #[test]
    fn nt_timers_and_io_completion_ports() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // NtCreateTimer + NtSetTimerEx(handle, &due, apc 0, mode 1, arg, resume, period 0, 0)
        call(&mut k, &mut bus, 0x00D7, &[0x4100_0000, 0, 0]);
        let t = bus.read_u32(0x4100_0000).unwrap() as u64;
        bus.write_u64(0x4100_0008, (-10_000i64) as u64).unwrap();
        assert_eq!(call(&mut k, &mut bus, 0x00FA, &[t, 0x4100_0008, 0, 1, 0, 0, 0, 0]), 0);
        let tp = k.objects.lookup(t as u32, 0).unwrap().guest_ptr;
        assert!(k.timers.is_set(tp));
        assert_eq!(call(&mut k, &mut bus, 0x00CD, &[t, 0x4100_0010]), 0);
        assert!(!k.timers.is_set(tp));
        // I/O completion port: empty -> the caller parks (or times out when polling)
        call(&mut k, &mut bus, 0x00D3, &[0x4100_0020, 0, 0, 0]);
        let port = bus.read_u32(0x4100_0020).unwrap() as u64;
        k.current_kthread = 0x4200_0000;
        bus.write_u64(0x4100_0030, 0).unwrap();
        assert_eq!(call(&mut k, &mut bus, 0x00F4, &[port, 0x4100_0040, 0x4100_0044, 0x4100_0048, 0x4100_0030]), 0x102);
        call(&mut k, &mut bus, 0x00F4, &[port, 0x4100_0040, 0x4100_0044, 0x4100_0048, 0]);
        let wait = k.pending_wait.take().expect("parked on the port");
        // NtSetIoCompletion queues a notification; the parked thread takes it
        assert_eq!(call(&mut k, &mut bus, 0x00F8, &[port, 0x11, 0x22, 0x33, 0x44]), 0);
        let status = match evaluate(&mut bus, &wait, 0) {
            WaitOutcome::Done(s) => k.complete_wait(&mut bus, &wait, s),
            WaitOutcome::Pending => panic!("still pending"),
        };
        assert_eq!(status, 0);
        let read = |bus: &MemoryBus, a: u64| bus.read_u32(a).unwrap();
        assert_eq!((read(&bus, 0x4100_0040), read(&bus, 0x4100_0044), read(&bus, 0x4100_0048), read(&bus, 0x4100_004C)), (0x11, 0x22, 0x33, 0x44));
        // two notifications queued: taken in order without waiting
        call(&mut k, &mut bus, 0x00F8, &[port, 1, 0, 0, 0]);
        call(&mut k, &mut bus, 0x00F8, &[port, 2, 0, 0, 0]);
        assert_eq!(call(&mut k, &mut bus, 0x00F4, &[port, 0x4100_0040, 0, 0, 0]), 0);
        assert_eq!(read(&bus, 0x4100_0040), 1);
        assert_eq!(call(&mut k, &mut bus, 0x00F4, &[port, 0x4100_0040, 0, 0, 0]), 0);
        assert_eq!(read(&bus, 0x4100_0040), 2);
        assert_eq!(call(&mut k, &mut bus, 0x00F4, &[0xF800_0400, 0, 0, 0, 0]), 0xC000_0008);
    }
}
