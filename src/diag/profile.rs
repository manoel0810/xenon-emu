//! XENON_PC_PROFILE (hot PCs and syscalls) and XENON_SYSCALL_SUMMARY (syscalls by caller), printed at exit.

use std::collections::HashMap;

#[derive(Default)]
pub struct Profiler {
    pc_hist: HashMap<(String, u64), u64>,
    sys_hist: HashMap<(String, u16), u64>,
    // (thread, lib, ordinal, lr) -> (count, first_frame, last_frame, last args)
    sys_agg: HashMap<(String, String, u16, u64), (u64, u64, u64, [u64; 4])>,
}

impl Profiler {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline(always)]
    pub fn record_pc(&mut self, thread: &str, pc: u64) {
        *self.pc_hist.entry((thread.to_string(), pc & 0xFFFF_FFFF)).or_insert(0) += 1;
    }

    #[inline(always)]
    pub fn record_syscall(&mut self, thread: &str, lib: &str, ordinal: u16) {
        *self.sys_hist.entry((format!("{}|{}", thread, lib), ordinal)).or_insert(0) += 1;
    }

    #[inline(always)]
    pub fn record_call(&mut self, thread: &str, lib: &str, ordinal: u16, lr: u64, frame: u64, args: &[u64]) {
        let e = self.sys_agg.entry((thread.to_string(), lib.to_string(), ordinal, lr & 0xFFFF_FFFF)).or_insert((0, frame, frame, [0; 4]));
        e.0 += 1; e.2 = frame; e.3 = [args[0], args[1], args[2], args[3]];
    }

    pub fn print_syscall_summary(&self) {
        let mut v: Vec<_> = self.sys_agg.iter().collect();
        v.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        println!("[SYSCALL SUMMARY] top 60 by count (thread | lib!ordinal | caller LR | count | frames first..last | last args)");
        for ((th, lib, ord, lr), (n, f0, f1, a)) in v.iter().take(60) {
            println!("  {:<28} {}!0x{:04X} lr=0x{:08X} x{:<6} f{}..{} a=[{:X},{:X},{:X},{:X}]", th, lib, ord, lr, n, f0, f1, a[0], a[1], a[2], a[3]);
        }
    }

    pub fn print_hot(&self) {
        let mut v: Vec<_> = self.pc_hist.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!("\n[PROFILE] Top 40 PCs (thread, pc, hits):");
        for ((n, pc), c) in v.iter().take(40) {
            println!("  {:<28} 0x{:08X} {}", n, pc, c);
        }
        let mut s: Vec<_> = self.sys_hist.iter().collect();
        s.sort_by(|a, b| b.1.cmp(a.1));
        println!("[PROFILE] Top 30 syscalls:");
        for ((n, o), c) in s.iter().take(30) {
            println!("  {:<40} #0x{:04X} {}", n, o, c);
        }
    }
}
