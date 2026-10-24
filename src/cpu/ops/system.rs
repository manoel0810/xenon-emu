//! Traps (tdi/twi/tw/td), sc, SPR/MSR/time base moves, CR field moves, cache and sync instructions.

use crate::cpu::interpreter::{Cpu, StepResult};
use crate::memory::MemoryBus;

impl Cpu {
    /// Primary opcodes: 2 | 3 | 17.
    #[inline(always)]
    pub(in crate::cpu) fn exec_system(&mut self, _bus: &mut MemoryBus, insn: u32, pc: u64, opcode: u32) -> Result<StepResult, String> {
        match opcode {
            // Opcode 2: tdi (Trap Doubleword Immediate)
            2 => {
                let to = (insn >> 21) & 0x1F;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i64;
                let a = self.regs.gpr[ra] as i64;
                let a_u = self.regs.gpr[ra];
                let simm_u = simm as u64;

                let cond = ((to & 0b10000) != 0 && a < simm)
                    || ((to & 0b01000) != 0 && a > simm)
                    || ((to & 0b00100) != 0 && a == simm)
                    || ((to & 0b00010) != 0 && a_u < simm_u)
                    || ((to & 0b00001) != 0 && a_u > simm_u);

                if cond {
                    return Err(format!("Trap condition met (tdi TO={} r{}={} SIMM={}) at PC=0x{:08X}", to, ra, a, simm, pc));
                }
            }

            // Opcode 3: twi (Trap Word Immediate)
            3 => {
                let to = (insn >> 21) & 0x1F;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i32;
                let a = self.regs.gpr[ra] as i32;
                let a_u = self.regs.gpr[ra] as u32;
                let simm_u = simm as u32;

                let cond = ((to & 0b10000) != 0 && a < simm)
                    || ((to & 0b01000) != 0 && a > simm)
                    || ((to & 0b00100) != 0 && a == simm)
                    || ((to & 0b00010) != 0 && a_u < simm_u)
                    || ((to & 0b00001) != 0 && a_u > simm_u);

                if cond {
                    return Err(format!("Trap condition met (twi TO={} r{}={} SIMM={}) at PC=0x{:08X}", to, ra, a, simm, pc));
                }
            }

            // Opcode 17: sc (System Call / HLE Trap)
            17 => {
                let lev = (insn >> 5) & 0x7F;
                return Ok(StepResult::Syscall { lev, pc });
            }

            _ => unreachable!("exec_system: opcode {} not in this group", opcode),
        }

        Ok(StepResult::Ok)
    }

    /// Opcode 31, xo: 83 | 146 | 178 | 339 | 371 | 467 | 1014 | 598 | 982 | 278 | 246 | 86 | 54 | 854 | 4 | 68 | 19 | 144.
    #[inline(always)]
    pub(in crate::cpu) fn exec_system_x31(&mut self, bus: &mut MemoryBus, insn: u32, pc: u64, xo: u32) -> Result<StepResult, String> {
        match xo {

            // 83: mfmsr (Move From Machine State Register)
            83 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                self.regs.gpr[rd] = self.regs.msr;
            }

            // 146: mtmsr (Move To Machine State Register)
            146 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                self.regs.msr = self.regs.gpr[rs];
            }

            // 178: mtmsrd (Move To Machine State Register Doubleword)
            178 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                self.regs.msr = self.regs.gpr[rs];
            }

            // mfspr (mflr, mfctr)
            339 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let spr_lo = (insn >> 16) & 0x1F;
                let spr_hi = (insn >> 11) & 0x1F;
                let spr = (spr_hi << 5) | spr_lo;
                match spr {
                    8 => self.regs.gpr[rd] = self.regs.lr,
                    9 => self.regs.gpr[rd] = self.regs.ctr,
                    268 => self.regs.gpr[rd] = (self.step_count as u64).wrapping_mul(50),
                    269 => self.regs.gpr[rd] = ((self.step_count as u64).wrapping_mul(50)) >> 32,
                    _ => return Err(format!("Unknown mfspr {}", spr)),
                }
            }

            // 371: mftb (Move From Time Base)
            371 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let tbr_lo = (insn >> 16) & 0x1F;
                let tbr_hi = (insn >> 11) & 0x1F;
                let tbr = (tbr_hi << 5) | tbr_lo;
                let tb = (self.step_count as u64).wrapping_mul(50);
                match tbr {
                    268 => self.regs.gpr[rd] = tb,
                    269 => self.regs.gpr[rd] = tb >> 32,
                    _ => self.regs.gpr[rd] = tb,
                }
            }

            // mtspr (mtlr, mtctr)
            467 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let spr_lo = (insn >> 16) & 0x1F;
                let spr_hi = (insn >> 11) & 0x1F;
                let spr = (spr_hi << 5) | spr_lo;
                match spr {
                    8 => self.regs.lr = self.regs.gpr[rs],
                    9 => self.regs.ctr = self.regs.gpr[rs],
                    _ => return Err(format!("Unknown mtspr {}", spr)),
                }
            }

            // 1014: dcbz (Data Cache Block Clear to Zero 128 bytes)
            1014 => {
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let ea = (if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) }) & !0x7F;
                for i in 0..128 {
                    bus.write_u8(ea + i as u64, 0)?;
                }
            }

            // 598: sync | 982: isync | 278: dcbt | 246: dcbtst | 86: dcbf | 54: dcbst | 854: eieio
            598 | 982 | 278 | 246 | 86 | 54 | 854 => {
                // Hardware memory barrier / cache hint stubs
            }

            // 4: tw (Trap Word)
            4 => {
                let to = (insn >> 21) & 0x1F;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i32;
                let b = self.regs.gpr[rb] as i32;
                let a_u = a as u32;
                let b_u = b as u32;

                let cond = ((to & 0b10000) != 0 && a < b)
                    || ((to & 0b01000) != 0 && a > b)
                    || ((to & 0b00100) != 0 && a == b)
                    || ((to & 0b00010) != 0 && a_u < b_u)
                    || ((to & 0b00001) != 0 && a_u > b_u);

                if cond {
                    return Err(format!("Trap condition met (tw TO={} r{}={} r{}={}) at PC=0x{:08X}", to, ra, a, rb, b, pc));
                }
            }

            // 68: td (Trap Doubleword)
            68 => {
                let to = (insn >> 21) & 0x1F;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i64;
                let b = self.regs.gpr[rb] as i64;
                let a_u = self.regs.gpr[ra];
                let b_u = self.regs.gpr[rb];

                let cond = ((to & 0b10000) != 0 && a < b)
                    || ((to & 0b01000) != 0 && a > b)
                    || ((to & 0b00100) != 0 && a == b)
                    || ((to & 0b00010) != 0 && a_u < b_u)
                    || ((to & 0b00001) != 0 && a_u > b_u);

                if cond {
                    return Err(format!("Trap condition met (td TO={} r{}={} r{}={}) at PC=0x{:08X}", to, ra, a, rb, b, pc));
                }
            }

            // 19: mfcr / mfocrf (Move From Condition Register [Field])
            19 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                if (insn & 0x0010_0000) != 0 {
                    // mfocrf: only the single selected field is moved; others read as 0
                    let crm = (insn >> 12) & 0xFF;
                    let mut mask = 0u32;
                    for i in 0..8 { if (crm >> (7 - i)) & 1 != 0 { mask |= 0xF << (28 - 4 * i); } }
                    self.regs.gpr[rd] = (self.regs.cr & mask) as u64;
                } else {
                    self.regs.gpr[rd] = self.regs.cr as u64;
                }
            }
            // 144: mtcrf / mtocrf (Move To Condition Register Fields)
            144 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let crm = (insn >> 12) & 0xFF;
                let mut mask = 0u32;
                for i in 0..8 { if (crm >> (7 - i)) & 1 != 0 { mask |= 0xF << (28 - 4 * i); } }
                self.regs.cr = (self.regs.cr & !mask) | ((self.regs.gpr[rs] as u32) & mask);
            }

            _ => unreachable!("exec_system_x31: xo {} not in this group", xo),
        }

        Ok(StepResult::Ok)
    }
}
