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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepResult {
    Ok,
    Syscall { lev: u32, pc: u64 },
    Halted,
}

pub struct Cpu {
    pub regs: Registers,
    pub halted: bool,
    pub step_count: u64,
}

impl Cpu {
    pub fn new(entry_point: u64) -> Self {
        let mut regs = Registers::new();
        regs.pc = entry_point;
        Self {
            regs,
            halted: false,
            step_count: 0,
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

        let opcode = (insn >> 26) & 0x3F;

        match opcode {
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

            // Opcode 17: sc (System Call / HLE Trap)
            17 => {
                let lev = (insn >> 5) & 0x7F;
                return Ok(StepResult::Syscall { lev, pc });
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
                let xo = (insn >> 1) & 0x3FF;
                let lk = insn & 0x1;
                match xo {
                    16 => {
                        let target = self.regs.lr;
                        if lk != 0 {
                            self.regs.lr = self.regs.pc;
                        }
                        self.regs.pc = target;
                    }
                    528 => {
                        let target = self.regs.ctr;
                        if lk != 0 {
                            self.regs.lr = self.regs.pc;
                        }
                        self.regs.pc = target;
                    }
                    _ => return Err(format!("Unimplemented opcode 19 xo {}", xo)),
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
                    self.regs.update_cr_cmpw(0, res as i32, 0);
                }
            }

            // Opcode 21: rlwinm / srwi / slwi
            21 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = (insn >> 11) & 0x1F;
                let mb = (insn >> 6) & 0x1F;
                let me = (insn >> 1) & 0x1F;

                let val = (self.regs.gpr[rs] as u32).rotate_left(sh);
                let mask = ppc_mask32(mb, me);
                self.regs.gpr[ra] = (val & mask) as u64;
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
                self.regs.update_cr_cmpw(0, res as i32, 0);
            }

            // Opcode 29: andis.
            29 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let uimm = ((insn & 0xFFFF) as u64) << 16;
                let res = self.regs.gpr[rs] & uimm;
                self.regs.gpr[ra] = res;
                self.regs.update_cr_cmpw(0, res as i32, 0);
            }

            // Opcode 30: rldicr / sldi
            30 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let sh = ((insn >> 11) & 0x1F) | (((insn >> 1) & 0x1) << 5);
                let me = ((insn >> 6) & 0x1F) | (((insn >> 5) & 0x1) << 5);
                let xo = (insn >> 2) & 0x7;

                if xo == 1 {
                    let rot = self.regs.gpr[rs].rotate_left(sh);
                    let mask = if me == 63 {
                        !0u64
                    } else {
                        !0u64 << (63 - me)
                    };
                    self.regs.gpr[ra] = rot & mask;
                } else {
                    return Err(format!("Unimplemented opcode 30 xo {}", xo));
                }
            }

            // Opcode 31: Extended opcodes (mr, mtctr, mflr, etc.)
            31 => {
                let xo = (insn >> 1) & 0x3FF;
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

                    // mr rA, rS
                    444 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        self.regs.gpr[ra] = self.regs.gpr[rs] | self.regs.gpr[rb];
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
                            _ => return Err(format!("Unknown mfspr {}", spr)),
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

                    // add
                    266 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        self.regs.gpr[rd] = self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]);
                    }

                    // subf
                    40 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        self.regs.gpr[rd] = self.regs.gpr[rb].wrapping_sub(self.regs.gpr[ra]);
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
                            self.regs.update_cr_cmpw(0, res as i32, 0);
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
                            self.regs.update_cr_cmpw(0, res as i32, 0);
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
                            self.regs.update_cr_cmpw(0, res as i32, 0);
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
                            self.regs.update_cr_cmpw(0, res as i32, 0);
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
                            self.regs.update_cr_cmpw(0, res as i32, 0);
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
                            self.regs.update_cr_cmpw(0, res as i32, 0);
                        }
                    }

                    // 824: srawi (Shift Right Algebraic Word Immediate)
                    824 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let sh = (insn >> 11) & 0x1F;
                        let val = self.regs.gpr[rs] as i32;
                        let res = (val >> sh) as i64 as u64;
                        self.regs.gpr[ra] = res;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, res as i32, 0);
                        }
                    }

                    // 598: sync | 982: isync | 278: dcbt | 86: dcbf | 1014: dcbz
                    598 | 982 | 278 | 86 | 1014 => {
                        // Hardware memory barrier / cache hint stubs
                    }

                    _ => return Err(format!("Unimplemented opcode 31 xo {}", xo)),
                }
            }

            // Opcode 32: lwz (Load Word and Zero)
            32 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let addr = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let val = bus.read_u32(addr)?;
                self.regs.gpr[rd] = val as u64;
            }

            // Opcode 34: lbz (Load Byte and Zero)
            34 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let addr = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let val = bus.read_u8(addr)?;
                self.regs.gpr[rd] = val as u64;
            }

            // Opcode 36: stw (Store Word)
            36 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                bus.write_u32(ea, self.regs.gpr[rs] as u32)?;
            }

            // Opcode 37: stwu (Store Word with Update)
            37 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                bus.write_u32(ea, self.regs.gpr[rs] as u32)?;
                self.regs.gpr[ra] = ea;
            }

            // Opcode 38: stb (Store Byte)
            38 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                bus.write_u8(ea, self.regs.gpr[rs] as u8)?;
            }

            // Opcode 40: lhz (Load Halfword and Zero)
            40 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let addr = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let val = bus.read_u16(addr)?;
                self.regs.gpr[rd] = val as u64;
            }

            // Opcode 42: lha (Load Halfword Algebraic)
            42 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let addr = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let val = bus.read_u16(addr)? as i16;
                self.regs.gpr[rd] = (val as i64) as u64;
            }

            // Opcode 44: sth (Store Halfword)
            44 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                bus.write_u16(ea, self.regs.gpr[rs] as u16)?;
            }

            // Opcode 58: ld / ldu / lwa (Load Doubleword)
            58 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let ds = ((insn & 0xFFFC) as i16 as i64) as u64;
                let xo = insn & 0x3;
                match xo {
                    0 => {
                        let ea = if ra == 0 { ds } else { self.regs.gpr[ra].wrapping_add(ds) };
                        let val = bus.read_u64(ea)?;
                        self.regs.gpr[rd] = val;
                    }
                    1 => {
                        let ea = self.regs.gpr[ra].wrapping_add(ds);
                        let val = bus.read_u64(ea)?;
                        self.regs.gpr[rd] = val;
                        self.regs.gpr[ra] = ea;
                    }
                    2 => {
                        let ea = if ra == 0 { ds } else { self.regs.gpr[ra].wrapping_add(ds) };
                        let val = bus.read_u32(ea)? as i32 as i64 as u64;
                        self.regs.gpr[rd] = val;
                    }
                    _ => return Err(format!("Unimplemented opcode 58 xo {}", xo)),
                }
            }

            // Opcode 62: std / stdu (Store Doubleword)
            62 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let ds = ((insn & 0xFFFC) as i16 as i64) as u64;
                let xo = insn & 0x3;
                match xo {
                    0 => {
                        let ea = if ra == 0 { ds } else { self.regs.gpr[ra].wrapping_add(ds) };
                        bus.write_u64(ea, self.regs.gpr[rs])?;
                    }
                    1 => {
                        let ea = self.regs.gpr[ra].wrapping_add(ds);
                        bus.write_u64(ea, self.regs.gpr[rs])?;
                        self.regs.gpr[ra] = ea;
                    }
                    _ => return Err(format!("Unimplemented opcode 62 xo {}", xo)),
                }
            }

            _ => return Err(format!("Unimplemented PowerPC opcode: {} (0x{:02X}) at PC=0x{:016X}", opcode, opcode, pc)),
        }

        Ok(StepResult::Ok)
    }
}
