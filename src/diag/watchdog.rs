//! XENON_WATCHDOG: lock-free progress counters read by a native monitor thread.
//! If the guest makes no progress for 4s the process exits with code 86 after printing where it was.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

static WD_STEPS: AtomicU64 = AtomicU64::new(0);
static WD_PC: AtomicU64 = AtomicU64::new(0);
static WD_LR: AtomicU64 = AtomicU64::new(0);
static WD_FRAME: AtomicU64 = AtomicU64::new(0);
static WD_SYS: AtomicU64 = AtomicU64::new(0); // (lib_is_xam << 32 | ordinal) while inside a syscall, 0 otherwise
static WD_A0: AtomicU64 = AtomicU64::new(0);
static WD_A1: AtomicU64 = AtomicU64::new(0);
static WD_A2: AtomicU64 = AtomicU64::new(0);
static WD_A3: AtomicU64 = AtomicU64::new(0);

/// Starts the monitor thread.
pub fn spawn() {
    std::thread::spawn(|| {
        let mut last = WD_STEPS.load(Relaxed);
        let mut still = 0u32;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            let now = WD_STEPS.load(Relaxed);
            if now == last && now != 0 { still += 1; } else { still = 0; last = now; }
            if still >= 8 {
                let s = WD_SYS.load(Relaxed);
                eprintln!("[WATCHDOG] NO PROGRESS for 4s: frame {} steps={} pc=0x{:08X} lr=0x{:08X} in_syscall={} (ordinal=0x{:04X}, xam={}) args=[{:X},{:X},{:X},{:X}]",
                    WD_FRAME.load(Relaxed), now, WD_PC.load(Relaxed) & 0xFFFF_FFFF, WD_LR.load(Relaxed) & 0xFFFF_FFFF,
                    s != 0, s & 0xFFFF, (s >> 32) != 0, WD_A0.load(Relaxed), WD_A1.load(Relaxed), WD_A2.load(Relaxed), WD_A3.load(Relaxed));
                std::process::exit(86);
            }
        }
    });
}

#[inline(always)]
pub fn set_frame(frame: u64) {
    WD_FRAME.store(frame, Relaxed);
}

#[inline(always)]
pub fn on_step(pc: u64, lr: u64) {
    WD_STEPS.fetch_add(1, Relaxed);
    WD_PC.store(pc, Relaxed);
    WD_LR.store(lr, Relaxed);
}

#[inline(always)]
pub fn enter_syscall(lib: &str, ordinal: u16, args: &[u64]) {
    WD_SYS.store(((lib.to_ascii_lowercase().contains("xam") as u64) << 32) | (ordinal as u64), Relaxed);
    WD_A0.store(args[0], Relaxed); WD_A1.store(args[1], Relaxed); WD_A2.store(args[2], Relaxed); WD_A3.store(args[3], Relaxed);
}

#[inline(always)]
pub fn leave_syscall() {
    WD_SYS.store(0, Relaxed);
}
