use crate::cpu::registers::Registers;
use crate::memory::MemoryBus;

/// PowerPC 32-bit mask generation according to PowerPC Architecture Manual
/// (Bit 0 is MSB, Bit 31 is LSB)
pub fn ppc_mask32(mb: u32, me: u32) -> u32 {
    let mut mask = 0u32;
    for i in 0..32 {
        let inside = if mb <= me {
            i >= mb && i <= me
        } else {
            i >= mb || i <= me
        };
        if inside {
            mask |= 1u32 << (31 - i);
        }
    }
    mask
}

/// PowerPC 64-bit mask generation according to PowerPC Architecture Manual
/// (Bit 0 is MSB, Bit 63 is LSB)
pub fn ppc_mask64(mb: u32, me: u32) -> u64 {
    let mut mask = 0u64;
    for i in 0..64 {
        let inside = if mb <= me {
            i >= mb && i <= me
        } else {
            i >= mb || i <= me
        };
        if inside {
            mask |= 1u64 << (63 - i);
        }
    }
    mask
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    Ok,
    Syscall { lev: u32, pc: u64 },
    Halted,
}

/// A lwarx/ldarx reservation: the address and the value it loaded (Xenia's RESERVED_LOAD keeps the value in
/// cached_reserve_value_). stwcx./stdcx. store only while it holds and memory still has that value, as Xenia's
/// RESERVED_STORE compare-exchange, and release it either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reservation {
    pub address: u64,
    pub value: u64,
    pub size: u8,
}

pub struct Cpu {
    pub regs: Registers,
    pub halted: bool,
    pub step_count: u64,
    pub reservation: Option<Reservation>,
}

impl Cpu {
    pub fn new(entry_point: u64) -> Self {
        let mut regs = Registers::new();
        regs.pc = entry_point;
        Self {
            regs,
            halted: false,
            step_count: 0,
            reservation: None,
        }
    }

    /// Fetches, decodes, and executes a single PowerPC instruction
    pub fn step(&mut self, bus: &mut MemoryBus) -> Result<StepResult, String> {
        if self.halted {
            return Ok(StepResult::Halted);
        }

        let pc = self.regs.pc;
        let insn = bus.read_u32(pc).map_err(|e| format!("Fetch error at 0x{:016X}: {}", pc, e))?;
        self.regs.pc += 4;
        self.step_count += 1;
        bus.timebase.tick();

        let opcode = (insn >> 26) & 0x3F;

        match opcode {
            7 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 20 | 21 | 23 | 24 | 25 | 26 | 27 | 28 | 29 | 30 => self.exec_int(bus, insn, pc, opcode),
            16 | 18 | 19 => self.exec_branch(bus, insn, pc, opcode),
            32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 40 | 41 | 42 | 43 | 44 | 45 | 48 | 49 | 50 | 51 | 52 | 53 | 54 | 55 | 58 | 62 => self.exec_loadstore(bus, insn, pc, opcode),
            59 | 63 => self.exec_float(bus, insn, pc, opcode),
            4 | 5 | 6 => self.exec_vmx(bus, insn, pc, opcode),
            2 | 3 | 17 => self.exec_system(bus, insn, pc, opcode),
            31 => {
                let xo = (insn >> 1) & 0x3FF;
                match xo {
                    0 | 32 | 444 | 412 | 476 | 284 | 266 | 10 | 522 | 138 | 650 | 202 | 714 | 8 | 520 | 136 | 648 | 104 | 40 | 26 | 24 | 28 | 60 | 124 | 316 | 536 | 824 | 792 | 27 | 539 | 794 | 826 | 827 | 922 | 954 | 986 | 235 | 747 | 233 | 745 | 75 | 587 | 11 | 523 | 459 | 971 | 491 | 1003 | 457 | 969 | 489 | 1001 | 73 | 9 | 58 | 234 | 746 | 232 | 744 | 200 | 712 => self.exec_int_x31(bus, insn, pc, xo),
                    20 | 84 | 150 | 214 | 23 | 87 | 279 | 21 | 151 | 215 | 407 | 149 | 535 | 599 | 663 | 727 | 983 | 55 | 119 | 311 | 373 | 375 | 53 | 341 | 343 | 183 | 247 | 439 | 181 | 534 | 790 | 532 | 662 | 918 | 660 | 567 | 631 | 695 | 759 => self.exec_loadstore_x31(bus, insn, pc, xo),
                    6 | 38 | 103 | 359 | 231 | 487 | 3 | 7 | 39 | 71 | 67 | 135 | 167 | 70 | 134 | 199 | 519 | 775 | 551 | 807 | 647 | 903 | 679 | 935 => self.exec_vmx_x31(bus, insn, pc, xo),
                    83 | 146 | 178 | 339 | 371 | 467 | 1014 | 598 | 982 | 278 | 246 | 86 | 54 | 854 | 4 | 68 | 19 | 144 => self.exec_system_x31(bus, insn, pc, xo),
                    _ => return Err(format!("Unimplemented opcode 31 xo {} (insn=0x{:08X})", xo, insn)),
                }
            }
            _ => return Err(format!("Unimplemented PowerPC opcode: {} (0x{:02X}) at PC=0x{:016X}", opcode, opcode, pc)),
        }
    }
}
