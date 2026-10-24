//! XENON_TRAP_TRACE: ring of the last executed instructions of the Metro primary thread, dumped on trap,
//! on the n-th watchpoint hit (XENON_WATCH_DUMP_HIT) or at a given frame (XENON_DUMP_RING_FRAME).

use std::collections::VecDeque;
use std::fmt::Display;

use crate::cpu::Registers;

const RING_CAPACITY: usize = 20000;

/// (pc, opcode, lr, ctr, r1, r3, r11, r12, r31, r30)
pub type TraceEntry = (u64, u32, u64, u64, u64, u64, u64, u64, u64, u64);

pub struct TrapRing {
    entries: VecDeque<TraceEntry>,
}

impl TrapRing {
    pub fn new() -> Self {
        Self { entries: VecDeque::with_capacity(64) }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records the instruction about to execute at `pc`.
    #[inline(always)]
    pub fn record(&mut self, pc: u64, op: u32, r: &Registers) {
        if self.entries.len() >= RING_CAPACITY { self.entries.pop_front(); }
        self.entries.push_back((pc, op, r.lr, r.ctr, r.gpr[1], r.gpr[3], r.gpr[11], r.gpr[12], r.gpr[31], r.gpr[30]));
    }

    fn print_seq_tail(&self, dump_n: usize) {
        let n = self.entries.len();
        for e in self.entries.iter().skip(n.saturating_sub(dump_n)) {
            println!("    seq pc=0x{:08X} op=0x{:08X} lr=0x{:08X} r3=0x{:X} r11=0x{:X} r30=0x{:X} r31=0x{:X}", e.0 & 0xFFFF_FFFF, e.1, e.2 & 0xFFFF_FFFF, e.5, e.6, e.9, e.8);
        }
    }

    /// XENON_WATCH_DUMP_HIT: the last `dump_n` instructions right after the watchpoint hit.
    pub fn dump_watch_hit(&self, hit: u64, pc: u64, dump_n: usize) {
        let n = self.entries.len();
        println!("  [WATCH DUMP] hit #{} by pc=0x{:08X}; last {} instructions:", hit, pc & 0xFFFF_FFFF, dump_n.min(n));
        self.print_seq_tail(dump_n);
    }

    /// Dump on trap: every entry where r31 changed plus the last 12 (all entries with `full`). Clears the ring.
    pub fn dump_trap(&mut self, err: &dyn Display, lr: u64, ctr: u64, full: bool) {
        println!("  [TRAP TRACE] last {} instructions before trap ({}); after-trap LR=0x{:08X} CTR=0x{:08X}:",
            self.entries.len(), err, lr, ctr);
        let tail_start = self.entries.len().saturating_sub(12);
        let mut prev31 = u64::MAX;
        for (i, (pc, op, lr, ctr, r1, r3, r11, r12, r31, _r30)) in self.entries.iter().enumerate() {
            let changed = *r31 != prev31;
            if changed || i >= tail_start || full {
                println!("    {}pc=0x{:08X} op=0x{:08X} lr=0x{:08X} ctr=0x{:08X} r1=0x{:08X} r3=0x{:08X} r11=0x{:08X} r12=0x{:08X} r31=0x{:08X}",
                    if changed { "[r31 changed] " } else { "" },
                    pc & 0xFFFF_FFFF, op, lr & 0xFFFF_FFFF, ctr & 0xFFFF_FFFF, r1 & 0xFFFF_FFFF, r3 & 0xFFFF_FFFF, r11 & 0xFFFF_FFFF, r12 & 0xFFFF_FFFF, r31 & 0xFFFF_FFFF);
            }
            prev31 = *r31;
        }
        self.entries.clear();
    }

    /// XENON_DUMP_RING_FRAME: distinct PCs by count + the last `dump_n` executed instructions, in order.
    pub fn dump_frame(&self, frame: u64, dump_n: usize) {
        let mut counts: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
        for e in self.entries.iter() { *counts.entry(e.0 & 0xFFFF_FFFF).or_insert(0) += 1; }
        println!("  [RING DUMP frame {}] {} instructions, {} distinct PCs", frame, self.entries.len(), counts.len());
        let mut v: Vec<(u64, u32)> = counts.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        for (pc, c) in v.iter().take(60) { println!("    hot pc=0x{:08X} x{}", pc, c); }
        self.print_seq_tail(dump_n);
    }
}
