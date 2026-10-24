//! Integer arithmetic, compare, logical, rotate and shift instructions.

use crate::cpu::interpreter::{ppc_mask32, ppc_mask64, Cpu, StepResult};
use crate::memory::MemoryBus;

impl Cpu {
    /// Primary opcodes: 7 | 8 | 10 | 11 | 12 | 13 | 14 | 15 | 20 | 21 | 23 | 24 | 25 | 26 | 27 | 28 | 29 | 30.
    #[inline(always)]
    pub(in crate::cpu) fn exec_int(&mut self, _bus: &mut MemoryBus, insn: u32, _pc: u64, opcode: u32) -> Result<StepResult, String> {
        match opcode {
            // Opcode 7: mulli (Multiply Low Immediate)
            7 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i64;
                let a = self.regs.gpr[ra] as i64;
                self.regs.gpr[rd] = a.wrapping_mul(simm) as u64;
            }

            // Opcode 8: subfic (Subtract from Immediate Carrying)
            8 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i64 as u64;
                let a = self.regs.gpr[ra];
                let (sum1, c1) = (!a).overflowing_add(simm);
                let (sum2, c2) = sum1.overflowing_add(1);
                let ca = if c1 || c2 { 1u64 } else { 0u64 };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[rd] = sum2;
            }

            // Opcode 10: cmpli (Compare Logical Immediate) / cmplwi
            10 => {
                let crf = (insn >> 23) & 0x7;
                let l = (insn >> 21) & 0x1;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = insn & 0xFFFF;
                if l == 0 {
                    let val = self.regs.gpr[ra] as u32;
                    self.regs.update_cr_cmplw(crf, val, uimm);
                } else {
                    let val = self.regs.gpr[ra];
                    self.regs.update_cr_cmplw(crf, (val >> 32) as u32, 0);
                }
            }

            // Opcode 11: cmpi (Compare Immediate) / cmpwi / cmpdi
            11 => {
                let crf = (insn >> 23) & 0x7;
                let l = (insn >> 21) & 0x1;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16;
                if l == 0 {
                    let val = self.regs.gpr[ra] as i32;
                    self.regs.update_cr_cmpw(crf, val, simm as i32);
                } else {
                    let val = self.regs.gpr[ra] as i64;
                    self.regs.update_cr_cmpd(crf, val, simm as i64);
                }
            }

            // Opcode 12/13: addic / addic. (Add Immediate Carrying [and Record]) - full 64-bit, CA = carry out of bit 0
            12 | 13 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i64 as u64;
                let (sum, carry) = self.regs.gpr[ra].overflowing_add(simm);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | ((carry as u64) << 29);
                self.regs.gpr[rd] = sum;
                if opcode == 13 {
                    self.regs.update_cr_cmpd(0, sum as i64, 0);
                }
            }
            // Opcode 14: addi / li
            14 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i64 as u64;
                if ra == 0 {
                    self.regs.gpr[rd] = simm;
                } else {
                    self.regs.gpr[rd] = self.regs.gpr[ra].wrapping_add(simm);
                }
            }

            // Opcode 15: addis / lis
            15 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = ((insn & 0xFFFF) as i16 as i64 as u64) << 16;
                if ra == 0 {
                    self.regs.gpr[rd] = simm;
                } else {
                    self.regs.gpr[rd] = self.regs.gpr[ra].wrapping_add(simm);
                }
            }

            // Opcode 20: rlwimi (Rotate Left Word Immediate then Mask Insert)
            20 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = (insn >> 11) & 0x1F;
                let mb = (insn >> 6) & 0x1F;
                let me = (insn >> 1) & 0x1F;
                let rc = insn & 1;

                let val = (self.regs.gpr[rs] as u32).rotate_left(sh);
                let mask = ppc_mask32(mb, me);
                let ra_val = self.regs.gpr[ra] as u32;
                let res = (val & mask) | (ra_val & !mask);
                self.regs.gpr[ra] = (self.regs.gpr[ra] & 0xFFFF_FFFF_0000_0000) | (res as u64);
                if rc != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // Opcode 21: rlwinm / srwi / slwi
            21 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = (insn >> 11) & 0x1F;
                let mb = (insn >> 6) & 0x1F;
                let me = (insn >> 1) & 0x1F;
                let rc = insn & 1;

                let val = (self.regs.gpr[rs] as u32).rotate_left(sh);
                let mask = ppc_mask32(mb, me);
                let res = val & mask;
                self.regs.gpr[ra] = res as u64;
                if rc != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // Opcode 23: rlwnm
            23 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let mb = (insn >> 6) & 0x1F;
                let me = (insn >> 1) & 0x1F;
                let rc = insn & 1;

                let sh = (self.regs.gpr[rb] & 0x1F) as u32;
                let val = (self.regs.gpr[rs] as u32).rotate_left(sh);
                let mask = ppc_mask32(mb, me);
                let res = val & mask;
                self.regs.gpr[ra] = res as u64;
                if rc != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // Opcode 24: ori / nop
            24 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = (insn & 0xFFFF) as u64;
                self.regs.gpr[ra] = self.regs.gpr[rs] | uimm;
            }

            // Opcode 25: oris
            25 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = ((insn & 0xFFFF) as u64) << 16;
                self.regs.gpr[ra] = self.regs.gpr[rs] | uimm;
            }

            // Opcode 26: xori
            26 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = (insn & 0xFFFF) as u64;
                self.regs.gpr[ra] = self.regs.gpr[rs] ^ uimm;
            }

            // Opcode 27: xoris
            27 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = ((insn & 0xFFFF) as u64) << 16;
                self.regs.gpr[ra] = self.regs.gpr[rs] ^ uimm;
            }

            // Opcode 28: andi.
            28 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = (insn & 0xFFFF) as u64;
                let res = self.regs.gpr[rs] & uimm;
                self.regs.gpr[ra] = res;
                self.regs.update_cr_cmpd(0, res as i64, 0);
            }

            // Opcode 29: andis.
            29 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = ((insn & 0xFFFF) as u64) << 16;
                let res = self.regs.gpr[rs] & uimm;
                self.regs.gpr[ra] = res;
                self.regs.update_cr_cmpd(0, res as i64, 0);
            }

            // Opcode 30: 64-bit Rotate and Mask Instructions (rldicl, rldicr, rldic, rldimi, rldcl, rldcr)
            30 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = (((insn >> 11) & 0x1F) | (((insn >> 1) & 0x1) << 5)) as u32;
                let m = (insn >> 5) & 0x3F; // MB or ME (bits 21..26)
                let xo = (insn >> 2) & 0x7;
                let rc = insn & 1;

                let res = match xo {
                    // 0: rldicl (Rotate Left Doubleword Immediate then Clear Left) / clrldi / srdi
                    0 => {
                        let rot = self.regs.gpr[rs].rotate_left(sh);
                        let mask = ppc_mask64(m, 63);
                        rot & mask
                    }
                    // 1: rldicr (Rotate Left Doubleword Immediate then Clear Right) / clrrdi / sldi
                    1 => {
                        let rot = self.regs.gpr[rs].rotate_left(sh);
                        let mask = ppc_mask64(0, m);
                        rot & mask
                    }
                    // 2: rldic (Rotate Left Doubleword Immediate then Clear)
                    2 => {
                        let rot = self.regs.gpr[rs].rotate_left(sh);
                        let me = 63u32.saturating_sub(sh);
                        let mask = ppc_mask64(m, me);
                        rot & mask
                    }
                    // 3: rldimi (Rotate Left Doubleword Immediate then Mask Insert)
                    3 => {
                        let rot = self.regs.gpr[rs].rotate_left(sh);
                        let me = 63u32.saturating_sub(sh);
                        let mask = ppc_mask64(m, me);
                        (rot & mask) | (self.regs.gpr[ra] & !mask)
                    }
                    // 4: rldcl (Rotate Left Doubleword then Clear Left)
                    4 => {
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let rsh = (self.regs.gpr[rb] & 0x3F) as u32;
                        let rot = self.regs.gpr[rs].rotate_left(rsh);
                        let mask = ppc_mask64(m, 63);
                        rot & mask
                    }
                    // 5: rldcr (Rotate Left Doubleword then Clear Right)
                    5 => {
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let rsh = (self.regs.gpr[rb] & 0x3F) as u32;
                        let rot = self.regs.gpr[rs].rotate_left(rsh);
                        let mask = ppc_mask64(0, m);
                        rot & mask
                    }
                    _ => return Err(format!("Unimplemented opcode 30 xo {}", xo)),
                };

                self.regs.gpr[ra] = res;
                if rc != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            _ => unreachable!("exec_int: opcode {} not in this group", opcode),
        }

        Ok(StepResult::Ok)
    }

    /// Opcode 31, xo: 0 | 32 | 444 | 412 | 476 | 284 | 266 | 10 | 522 | 138 | 650 | 202 | 714 | 8 | 520 | 136 | 648 | 104 | 40 | 26 | 24 | 28 | 60 | 124 | 316 | 536 | 824 | 792 | 27 | 539 | 794 | 826 | 827 | 922 | 954 | 986 | 235 | 747 | 233 | 745 | 75 | 587 | 11 | 523 | 459 | 971 | 491 | 1003 | 457 | 969 | 489 | 1001 | 73 | 9 | 58 | 234 | 746 | 232 | 744 | 200 | 712.
    #[inline(always)]
    pub(in crate::cpu) fn exec_int_x31(&mut self, _bus: &mut MemoryBus, insn: u32, _pc: u64, xo: u32) -> Result<StepResult, String> {
        match xo {
            // 0: cmp (Compare)
            0 => {
                let crf = ((insn >> 23) & 0x7) as u32;
                let l = (insn >> 21) & 1;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                if l == 0 {
                    self.regs.update_cr_cmpw(crf, self.regs.gpr[ra] as i32, self.regs.gpr[rb] as i32);
                } else {
                    self.regs.update_cr_cmpd(crf, self.regs.gpr[ra] as i64, self.regs.gpr[rb] as i64);
                }
            }

            // 32: cmpl (Compare Logical)
            32 => {
                let crf = ((insn >> 23) & 0x7) as u32;
                let l = (insn >> 21) & 1;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                if l == 0 {
                    self.regs.update_cr_cmplw(crf, self.regs.gpr[ra] as u32, self.regs.gpr[rb] as u32);
                } else {
                    let a = self.regs.gpr[ra];
                    let b = self.regs.gpr[rb];
                    let shift = (7 - crf) * 4;
                    let mask = !(0xFu32 << shift);
                    let bits = if a < b { 0b1000 } else if a > b { 0b0100 } else { 0b0010 };
                    self.regs.cr = (self.regs.cr & mask) | (bits << shift);
                }
            }

            // 444: or / mr (Move Register)
            444 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[rs] | self.regs.gpr[rb];
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 412: orc (OR with Complement)
            412 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[rs] | (!self.regs.gpr[rb]);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 476: nand (NAND)
            476 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = !(self.regs.gpr[rs] & self.regs.gpr[rb]);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 284: eqv (Equivalent)
            284 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = !(self.regs.gpr[rs] ^ self.regs.gpr[rb]);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // add / add.
            266 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 10 | 522: addc (Add Carrying) - 64-bit
            10 | 522 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let (res, c) = self.regs.gpr[ra].overflowing_add(self.regs.gpr[rb]);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | ((c as u64) << 29);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 138 | 650: adde (Add Extended) - 64-bit
            138 | 650 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let cin = (self.regs.xer >> 29) & 1;
                let (s1, c1) = self.regs.gpr[ra].overflowing_add(self.regs.gpr[rb]);
                let (res, c2) = s1.overflowing_add(cin);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (((c1 || c2) as u64) << 29);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 202 | 714: addze (Add to Zero Extended) - 64-bit
            202 | 714 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let cin = (self.regs.xer >> 29) & 1;
                let (res, c) = self.regs.gpr[ra].overflowing_add(cin);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | ((c as u64) << 29);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 8 | 520: subfc (Subtract From Carrying) - 64-bit: rd = rb - ra, CA = no borrow
            8 | 520 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra];
                let b = self.regs.gpr[rb];
                let (s1, c1) = (!a).overflowing_add(b);
                let (res, c2) = s1.overflowing_add(1);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (((c1 || c2) as u64) << 29);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 136 | 648: subfe (Subtract From Extended) - 64-bit: rd = ~ra + rb + CA
            136 | 648 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let cin = (self.regs.xer >> 29) & 1;
                let (s1, c1) = (!self.regs.gpr[ra]).overflowing_add(self.regs.gpr[rb]);
                let (res, c2) = s1.overflowing_add(cin);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (((c1 || c2) as u64) << 29);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 104: neg (Negate)
            104 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let res = (0u64).wrapping_sub(self.regs.gpr[ra]);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // subf
            40 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[rb].wrapping_sub(self.regs.gpr[ra]);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 26: cntlzw (Count Leading Zeros Word)
            26 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let val = self.regs.gpr[rs] as u32;
                let lz = val.leading_zeros() as u64;
                self.regs.gpr[ra] = lz;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpw(0, lz as i32, 0);
                }
            }

            // 24: slw (Shift Left Word)
            24 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let shift = (self.regs.gpr[rb] & 0x3F) as u32;
                let res = if shift < 32 {
                    ((self.regs.gpr[rs] as u32) << shift) as u64
                } else {
                    0
                };
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 28: and / and.
            28 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[rs] & self.regs.gpr[rb];
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 60: andc
            60 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[rs] & (!self.regs.gpr[rb]);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 124: nor
            124 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = !(self.regs.gpr[rs] | self.regs.gpr[rb]);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 316: xor
            316 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = self.regs.gpr[rs] ^ self.regs.gpr[rb];
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 536: srw (Shift Right Word)
            536 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let shift = (self.regs.gpr[rb] & 0x3F) as u32;
                let res = if shift < 32 {
                    ((self.regs.gpr[rs] as u32) >> shift) as u64
                } else {
                    0
                };
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 824: srawi (Shift Right Algebraic Word Immediate)
            824 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = (insn >> 11) & 0x1F;
                let val = self.regs.gpr[rs] as i32;
                let res = (val >> sh) as i64 as u64;
                let ca = if sh > 0 && val < 0 && (val as u32 & ((1u32 << sh) - 1)) != 0 {
                    1
                } else {
                    0
                };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 792: sraw (Shift Right Algebraic Word)
            792 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let shift = (self.regs.gpr[rb] & 0x3F) as u32;
                let val = self.regs.gpr[rs] as i32;
                let res = if shift < 32 {
                    (val >> shift) as i64 as u64
                } else if val < 0 {
                    !0u64
                } else {
                    0u64
                };
                let ca = if shift > 0 && shift < 32 && val < 0 && (val as u32 & ((1u32 << shift) - 1)) != 0 {
                    1
                } else if shift >= 32 && val < 0 {
                    1
                } else {
                    0
                };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 27: sld (Shift Left Doubleword)
            27 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let shift = (self.regs.gpr[rb] & 0x7F) as u32;
                let res = if shift < 64 {
                    self.regs.gpr[rs] << shift
                } else {
                    0
                };
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 539: srd (Shift Right Doubleword)
            539 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let shift = (self.regs.gpr[rb] & 0x7F) as u32;
                let res = if shift < 64 {
                    self.regs.gpr[rs] >> shift
                } else {
                    0
                };
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 794: srad (Shift Right Algebraic Doubleword)
            794 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let shift = (self.regs.gpr[rb] & 0x7F) as u32;
                let val = self.regs.gpr[rs] as i64;
                let res = if shift < 64 {
                    (val >> shift) as u64
                } else if val < 0 {
                    !0u64
                } else {
                    0u64
                };
                let ca = if shift > 0 && shift < 64 && val < 0 && (val as u64 & ((1u64 << shift) - 1)) != 0 {
                    1
                } else if shift >= 64 && val < 0 {
                    1
                } else {
                    0
                };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 826 / 827: sradi (Shift Right Algebraic Doubleword Immediate)
            826 | 827 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = ((insn >> 11) & 0x1F) | (((insn >> 1) & 0x1) << 5);
                let val = self.regs.gpr[rs] as i64;
                let res = (val >> sh) as u64;
                let ca = if sh > 0 && val < 0 && (val as u64 & ((1u64 << sh) - 1)) != 0 {
                    1
                } else {
                    0
                };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 922: extsh (Extend Sign Halfword)
            922 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let res = (self.regs.gpr[rs] as i16 as i64) as u64;
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 954: extsb (Extend Sign Byte)
            954 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let res = (self.regs.gpr[rs] as i8 as i64) as u64;
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 986: extsw (Extend Sign Word)
            986 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let res = (self.regs.gpr[rs] as i32 as i64) as u64;
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, res as i64, 0);
                }
            }

            // 235 | 747: mullw (Multiply Low Word)
            235 | 747 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i32;
                let b = self.regs.gpr[rb] as i32;
                let prod = a.wrapping_mul(b);
                self.regs.gpr[rd] = prod as i64 as u64;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpw(0, prod, 0);
                }
            }

            // 233 | 745: mulld (Multiply Low Doubleword)
            233 | 745 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i64;
                let b = self.regs.gpr[rb] as i64;
                let prod = a.wrapping_mul(b);
                self.regs.gpr[rd] = prod as u64;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, prod, 0);
                }
            }

            // 75 | 587: mulhw (Multiply High Word)
            75 | 587 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i32 as i64;
                let b = self.regs.gpr[rb] as i32 as i64;
                let prod = a.wrapping_mul(b);
                self.regs.gpr[rd] = ((prod >> 32) as i32) as i64 as u64;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpw(0, (prod >> 32) as i32, 0);
                }
            }

            // 11 | 523: mulhwu (Multiply High Word Unsigned)
            11 | 523 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as u32 as u64;
                let b = self.regs.gpr[rb] as u32 as u64;
                let prod = a.wrapping_mul(b);
                self.regs.gpr[rd] = (prod >> 32) as u64;
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpw(0, (prod >> 32) as i32, 0);
                }
            }

            // 459 | 971: divwu (Divide Word Unsigned)
            459 | 971 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as u32;
                let b = self.regs.gpr[rb] as u32;
                if b == 0 {
                    self.regs.gpr[rd] = 0;
                } else {
                    self.regs.gpr[rd] = (a / b) as u64;
                }
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                }
            }

            // 491 | 1003: divw (Divide Word Signed)
            491 | 1003 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i32;
                let b = self.regs.gpr[rb] as i32;
                if b == 0 || (a == i32::MIN && b == -1) {
                    self.regs.gpr[rd] = 0;
                } else {
                    self.regs.gpr[rd] = (a / b) as u32 as u64;
                }
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                }
            }

            // 457 | 969: divdu (Divide Doubleword Unsigned)
            457 | 969 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra];
                let b = self.regs.gpr[rb];
                if b == 0 {
                    self.regs.gpr[rd] = 0;
                } else {
                    self.regs.gpr[rd] = a / b;
                }
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, self.regs.gpr[rd] as i64, 0);
                }
            }

            // 489 | 1001: divd (Divide Doubleword Signed)
            489 | 1001 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let a = self.regs.gpr[ra] as i64;
                let b = self.regs.gpr[rb] as i64;
                if b == 0 || (a == i64::MIN && b == -1) {
                    self.regs.gpr[rd] = 0;
                } else {
                    self.regs.gpr[rd] = (a / b) as u64;
                }
                if (insn & 1) != 0 {
                    self.regs.update_cr_cmpd(0, self.regs.gpr[rd] as i64, 0);
                }
            }
            // ---- Integer arithmetic gaps ----
            // 73: mulhd | 9: mulhdu
            73 | 9 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let rb = ((insn >> 11) & 0x1F) as usize;
                let res = if xo == 73 {
                    (((self.regs.gpr[ra] as i64 as i128) * (self.regs.gpr[rb] as i64 as i128)) >> 64) as u64
                } else {
                    (((self.regs.gpr[ra] as u128) * (self.regs.gpr[rb] as u128)) >> 64) as u64
                };
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 { self.regs.update_cr_cmpd(0, res as i64, 0); }
            }
            // 58: cntlzd
            58 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let res = self.regs.gpr[rs].leading_zeros() as u64;
                self.regs.gpr[ra] = res;
                if (insn & 1) != 0 { self.regs.update_cr_cmpd(0, res as i64, 0); }
            }
            // 234 | 746: addme | 232 | 744: subfme | 200 | 712: subfze
            234 | 746 | 232 | 744 | 200 | 712 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let cin = (self.regs.xer >> 29) & 1;
                let a = self.regs.gpr[ra];
                // addme: a + CA - 1 ; subfme: ~a + CA - 1 ; subfze: ~a + CA
                let (base, addend) = match xo & 0x1FF {
                    234 => (a, u64::MAX),
                    232 => (!a, u64::MAX),
                    _ => (!a, 0),
                };
                let (s1, c1) = base.overflowing_add(addend);
                let (res, c2) = s1.overflowing_add(cin);
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (((c1 || c2) as u64) << 29);
                self.regs.gpr[rd] = res;
                if (insn & 1) != 0 { self.regs.update_cr_cmpd(0, res as i64, 0); }
            }

            _ => unreachable!("exec_int_x31: xo {} not in this group", xo),
        }

        Ok(StepResult::Ok)
    }
}
