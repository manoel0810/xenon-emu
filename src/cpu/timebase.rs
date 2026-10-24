//! Guest time base (TB, read by `mftb` / `mfspr 268,269`), shared by every guest thread.
//!
//! On the console TB is a single 50 MHz counter all hardware threads read; Xenia reads it from one clock too
//! (Xenia Canary 451f9155, ppc_emit_control.cc `InstrEmit_mftb` -> `LoadClock` -> `Clock::QueryGuestTickCount`,
//! frequency `Clock::guest_tick_frequency()` = 50 MHz, the value KeQueryPerformanceFrequency returns).
//! Xenia drives it from host time; here it is driven by guest execution so runs stay deterministic: each emulated
//! frame (one vblank, 1/60 s) spans `TICKS_PER_FRAME` ticks, spread over the instructions all threads execute in
//! that frame. It never goes backwards.

pub const TICKS_PER_SECOND: u64 = 50_000_000;
pub const FRAMES_PER_SECOND: u64 = 60;
pub const TICKS_PER_FRAME: u64 = TICKS_PER_SECOND / FRAMES_PER_SECOND;

#[derive(Debug, Clone)]
pub struct TimeBase {
    frame_start: u64,
    steps: u64,
    steps_per_frame: u64,
}

impl Default for TimeBase {
    fn default() -> Self {
        Self { frame_start: 0, steps: 0, steps_per_frame: 25_000 }
    }
}

impl TimeBase {
    /// Starts frame `frame` (vblank count), in which the scheduler expects to run about `steps_per_frame`
    /// instructions across all threads.
    pub fn begin_frame(&mut self, frame: u64, steps_per_frame: u64) {
        self.frame_start = (frame * TICKS_PER_FRAME).max(self.now());
        self.steps = 0;
        self.steps_per_frame = steps_per_frame.max(1);
    }

    /// One guest instruction executed (by any thread).
    pub fn tick(&mut self) {
        self.steps += 1;
    }

    /// Current TB value. Within a frame it advances linearly with the instructions executed and stops at the
    /// next frame's start if the frame runs more instructions than expected.
    pub fn now(&self) -> u64 {
        let steps = self.steps.min(self.steps_per_frame);
        self.frame_start + steps * TICKS_PER_FRAME / self.steps_per_frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_frame_is_one_sixtieth_of_a_second() {
        let mut tb = TimeBase::default();
        tb.begin_frame(10, 4);
        assert_eq!(tb.now(), 10 * 833_333);
        tb.tick();
        tb.tick();
        assert_eq!(tb.now(), 10 * 833_333 + 833_333 / 2);
        for _ in 0..10 {
            tb.tick();
        }
        assert_eq!(tb.now(), 11 * 833_333, "stops at the next frame");
        tb.begin_frame(11, 4);
        assert_eq!(tb.now(), 11 * 833_333);
    }

    #[test]
    fn never_goes_backwards() {
        let mut tb = TimeBase::default();
        tb.begin_frame(5, 2);
        tb.tick();
        let t = tb.now();
        tb.begin_frame(3, 2); // a frame number lower than the time already seen
        assert!(tb.now() >= t);
        let mut last = tb.now();
        for f in 6..20 {
            tb.begin_frame(f, 1 + f % 3);
            for _ in 0..f {
                assert!(tb.now() >= last);
                last = tb.now();
                tb.tick();
            }
        }
    }

    #[test]
    fn all_threads_read_the_same_clock() {
        // two CPUs on one bus see one counter: what one thread reads after another ran is never earlier
        let mut bus = crate::memory::MemoryBus::new();
        bus.timebase.begin_frame(1, 8);
        let mut a = crate::cpu::Cpu::new(0x1000);
        let mut b = crate::cpu::Cpu::new(0x2000);
        // mftb r3 (tbr 268) ; mftb r4 (tbr 268)
        bus.write_u32(0x1000, 0x7C6C_42E6).unwrap();
        bus.write_u32(0x2000, 0x7C8C_42E6).unwrap();
        a.step(&mut bus).unwrap();
        for _ in 0..3 {
            bus.timebase.tick();
        }
        b.step(&mut bus).unwrap();
        assert_eq!(a.regs.gpr[3], TICKS_PER_FRAME + TICKS_PER_FRAME / 8, "mftb counts its own instruction");
        assert_eq!(b.regs.gpr[4], TICKS_PER_FRAME + 5 * TICKS_PER_FRAME / 8);
    }
}
