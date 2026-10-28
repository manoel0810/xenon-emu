// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/kernel_state.h     X_TIME_STAMP_BUNDLE
//   src/xenia/kernel/kernel_state.cc    CreateKeTimestampBundle (address 0x80240EE0), UpdateKeTimestampBundle
//   src/xenia/kernel/xboxkrnl/xboxkrnl_module.cc   SetVariableMapping(KeTimeStampBundle)
//
//! KeTimeStampBundle: the kernel variable titles read the current time from without a call (the dash's XUI
//! timers use its tick_count). Xenia refreshes it every millisecond of host time; here it follows the guest time
//! base and is refreshed before each thread's time slice, so a thread sees it advance between slices.

use super::scheduler::system_time;
use crate::memory::MemoryBus;

/// (xboxkrnl ordinal, guest address) of KeTimeStampBundle.
pub const KE_TIME_STAMP_BUNDLE: (u16, u32) = (0x00AD, 0x8024_0EE0);

/// Time base ticks (50 MHz) per millisecond and per 100 ns interrupt time unit (KeQueryInterruptTime).
const TICKS_PER_MS: u64 = 50_000;
const TICKS_PER_100NS: u64 = 5;

/// UpdateKeTimestampBundle: X_TIME_STAMP_BUNDLE {interrupt_time u64, system_time u64, tick_count u32, padding u32},
/// big-endian, for the time base value `now`.
pub fn update(bus: &mut MemoryBus, now: u64) {
    let base = KE_TIME_STAMP_BUNDLE.1 as u64;
    let _ = bus.write_u64(base, now / TICKS_PER_100NS);
    let _ = bus.write_u64(base + 8, system_time(now));
    let _ = bus.write_u32(base + 0x10, (now / TICKS_PER_MS) as u32);
}

/// CreateKeTimestampBundle: the first values and a zero padding.
pub fn create(bus: &mut MemoryBus) -> (u16, u32) {
    let now = bus.timebase.now();
    update(bus, now);
    let _ = bus.write_u32(KE_TIME_STAMP_BUNDLE.1 as u64 + 0x14, 0);
    KE_TIME_STAMP_BUNDLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_follows_the_time_base() {
        let mut bus = MemoryBus::new();
        let (ordinal, addr) = create(&mut bus);
        assert_eq!((ordinal, addr), (0x00AD, 0x8024_0EE0));
        let a = addr as u64;
        assert_eq!((bus.read_u64(a).unwrap(), bus.read_u32(a + 0x10).unwrap(), bus.read_u32(a + 0x14).unwrap()), (0, 0, 0));
        assert_eq!(bus.read_u64(a + 8).unwrap(), system_time(0));
        // 90 frames = 1.5 s: tick_count in milliseconds, interrupt time in 100 ns, system time from boot
        bus.timebase.begin_frame(90, 1);
        let now = bus.timebase.now();
        update(&mut bus, now);
        assert_eq!(bus.read_u32(a + 0x10).unwrap(), 1_499);
        assert_eq!(bus.read_u64(a).unwrap(), now / 5);
        assert_eq!(bus.read_u64(a + 8).unwrap(), system_time(now));
    }
}
