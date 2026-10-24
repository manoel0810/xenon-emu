//! b, bc, bclr/bcctr and the CR-logical group of opcode 19.

use crate::cpu::interpreter::{Cpu, StepResult};
use crate::memory::MemoryBus;

impl Cpu {
    /// Primary opcodes: 16 | 18 | 19.
    #[inline(always)]
    pub(in crate::cpu) fn exec_branch(&mut self, _bus: &mut MemoryBus, insn: u32, pc: u64, opcode: u32) -> Result<StepResult, String> {
        match opcode {

            // Opcode 16: bc / bdnz / bgt / blt / beq
            16 => {
                let bo = (insn >> 21) & 0x1F;
                let bi = (insn >> 16) & 0x1F;
                let bd = (insn & 0xFFFC) as i16 as i64;
                let aa = (insn >> 1) & 0x1;
                let lk = insn & 0x1;

                let target = if aa != 0 {
                    bd as u64
                } else {
                    (pc as i64 + bd) as u64
                };

                // CTR decrement check (if BO[2] == 0)
                let ctr_ok = if (bo & 0b00100) == 0 {
                    self.regs.ctr = self.regs.ctr.wrapping_sub(1);
                    if (bo & 0b00010) != 0 {
                        self.regs.ctr == 0
                    } else {
                        self.regs.ctr != 0
                    }
                } else {
                    true
                };

                // Condition check (if BO[4] == 0)
                let cond_ok = if (bo & 0b10000) == 0 {
                    let cr_bit = self.regs.get_cr_bit(bi);
                    let test_val = ((bo >> 3) & 1) != 0;
                    cr_bit == test_val
                } else {
                    true
                };

                if ctr_ok && cond_ok {
                    if lk != 0 {
                        self.regs.lr = self.regs.pc;
                    }
                    self.regs.pc = target;
                }
            }

            // Opcode 18: b / bl / ba / bla
            18 => {
                let li = (insn & 0x03FF_FFFC) as i32;
                let li = ((li << 6) >> 6) as i64;
                let aa = (insn >> 1) & 0x1;
                let lk = insn & 0x1;

                if lk != 0 {
                    self.regs.lr = self.regs.pc;
                }

                let target = if aa != 0 {
                    li as u64
                } else {
                    (pc as i64 + li) as u64
                };
                self.regs.pc = target;
            }

            // Opcode 19: bclr / blrl / bcctr / bctrl
            19 => {
                let bo = (insn >> 21) & 0x1F;
                let bi = (insn >> 16) & 0x1F;
                let xo = (insn >> 1) & 0x3FF;
                let lk = insn & 0x1;

                // CTR decrement check (only for bclr xo 16 when BO[2] == 0)
                let ctr_ok = if xo == 16 && (bo & 0b00100) == 0 {
                    self.regs.ctr = self.regs.ctr.wrapping_sub(1);
                    if (bo & 0b00010) != 0 {
                        self.regs.ctr == 0
                    } else {
                        self.regs.ctr != 0
                    }
                } else {
                    true
                };

                // Condition check (if BO[0] == 0, i.e., bit 4 from right is 0)
                let cond_ok = if (bo & 0b10000) == 0 {
                    let cr_bit = self.regs.get_cr_bit(bi);
                    let test_val = ((bo >> 3) & 1) != 0;
                    cr_bit == test_val
                } else {
                    true
                };

                if ctr_ok && cond_ok {
                    let target = match xo {
                        16 => self.regs.lr,
                        528 => self.regs.ctr,
                        _ => return Err(format!("Unimplemented opcode 19 xo {}", xo)),
                    };
                    if lk != 0 {
                        self.regs.lr = self.regs.pc;
                    }
                    self.regs.pc = target;
                }
            }

            _ => unreachable!("exec_branch: opcode {} not in this group", opcode),
        }

        Ok(StepResult::Ok)
    }
}
