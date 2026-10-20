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

            // Opcode 4: AltiVec / VMX128 Vector Instructions
            4 => {
                // Check VA-form first (bits 26..31 == 43: vperm)
                if (insn & 0x3F) == 43 {
                    let vd = ((insn >> 21) & 0x1F) as usize;
                    let va = ((insn >> 16) & 0x1F) as usize;
                    let vb = ((insn >> 11) & 0x1F) as usize;
                    let vc = ((insn >> 6) & 0x1F) as usize;
                    let mut res = [0u8; 16];
                    for i in 0..16 {
                        let sel = (self.regs.vr[vc][i] & 0x1F) as usize;
                        res[i] = if sel < 16 {
                            self.regs.vr[va][sel]
                        } else {
                            self.regs.vr[vb][sel - 16]
                        };
                    }
                    self.regs.vr[vd] = res;
                } else if (insn & 3) == 3 {
                    // VMX128 Memory and Element Instructions (bits 30..31 == 11)
                    let op7 = (insn >> 4) & 0x7F;
                    let vdh = (insn >> 2) & 0x3;
                    let vd128 = (insn >> 21) & 0x1F;
                    let vr = ((vdh << 5) | vd128) as usize;
                    let ra = ((insn >> 16) & 0x1F) as usize;
                    let rb = ((insn >> 11) & 0x1F) as usize;

                    match op7 {
                        // 12 | 44: lvx128 / lvx128l (Load Vector 128-bit Indexed)
                        12 | 44 => {
                            let ea = (if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) }) & !0xF;
                            for i in 0..16 {
                                self.regs.vr[vr][i] = bus.read_u8(ea + i as u64)?;
                            }
                        }

                        // 28 | 60: stvx128 / stvx128l (Store Vector 128-bit Indexed)
                        28 | 60 => {
                            let ea = (if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) }) & !0xF;
                            for i in 0..16 {
                                bus.write_u8(ea + i as u64, self.regs.vr[vr][i])?;
                            }
                        }

                        // 8: lvewx128 (Load Vector Element Word Indexed)
                        8 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let idx = (ea & 0xC) as usize;
                            let bytes = bus.read_u32(ea & !3)?.to_be_bytes();
                            self.regs.vr[vr][idx..idx + 4].copy_from_slice(&bytes);
                        }

                        // 24: stvewx128 (Store Vector Element Word Indexed)
                        24 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let idx = (ea & 0xC) as usize;
                            let val = u32::from_be_bytes([
                                self.regs.vr[vr][idx],
                                self.regs.vr[vr][idx + 1],
                                self.regs.vr[vr][idx + 2],
                                self.regs.vr[vr][idx + 3],
                            ]);
                            bus.write_u32(ea & !3, val)?;
                        }

                        // 64 | 96: lvlx128 / lvlxl128 (Load Vector Left Indexed)
                        64 | 96 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let eb = (ea & 0xF) as usize;
                            let aligned_ea = ea & !0xF;
                            let mut res = [0u8; 16];
                            for i in 0..(16 - eb) {
                                res[i] = bus.read_u8(aligned_ea + (eb + i) as u64)?;
                            }
                            self.regs.vr[vr] = res;
                        }

                        // 68 | 100: lvrx128 / lvrxl128 (Load Vector Right Indexed)
                        68 | 100 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let eb = (ea & 0xF) as usize;
                            let mut res = [0u8; 16];
                            if eb != 0 {
                                let aligned_ea = ea & !0xF;
                                for i in 0..eb {
                                    res[16 - eb + i] = bus.read_u8(aligned_ea + i as u64)?;
                                }
                            }
                            self.regs.vr[vr] = res;
                        }

                        // 80 | 112: stvlx128 / stvlxl128 (Store Vector Left Indexed)
                        80 | 112 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let eb = (ea & 0xF) as usize;
                            let aligned_ea = ea & !0xF;
                            for i in 0..(16 - eb) {
                                bus.write_u8(aligned_ea + (eb + i) as u64, self.regs.vr[vr][i])?;
                            }
                        }

                        // 84 | 116: stvrx128 / stvrxl128 (Store Vector Right Indexed)
                        84 | 116 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let eb = (ea & 0xF) as usize;
                            if eb != 0 {
                                let aligned_ea = ea & !0xF;
                                for i in 0..eb {
                                    bus.write_u8(aligned_ea + i as u64, self.regs.vr[vr][16 - eb + i])?;
                                }
                            }
                        }

                        _ => return Err(format!("Unimplemented VMX128 opcode 4 op7={} (insn=0x{:08X}) at PC=0x{:08X}", op7, insn, pc)),
                    }
                } else {
                    let xo = insn & 0x7FF;
                    match xo {
                        // 780 (0x30C): vspltisb (Vector Splat Immediate Signed Byte)
                        780 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let simm = ((insn >> 16) & 0x1F) as i8;
                            let simm = if simm & 0x10 != 0 { simm | !0x1F } else { simm } as u8;
                            self.regs.vr[vd].fill(simm);
                        }

                        // 844 (0x34C): vspltish (Vector Splat Immediate Signed Halfword)
                        844 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let simm = ((insn >> 16) & 0x1F) as i8;
                            let simm = if simm & 0x10 != 0 { simm | !0x1F } else { simm } as i16;
                            let bytes = simm.to_be_bytes();
                            for h in 0..8 {
                                self.regs.vr[vd][h * 2] = bytes[0];
                                self.regs.vr[vd][h * 2 + 1] = bytes[1];
                            }
                        }

                        // 908 (0x38C): vspltisw (Vector Splat Immediate Signed Word)
                        908 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let simm = ((insn >> 16) & 0x1F) as i8;
                            let simm = if simm & 0x10 != 0 { simm | !0x1F } else { simm } as i32;
                            let bytes = simm.to_be_bytes();
                            for w in 0..4 {
                                self.regs.vr[vd][w * 4..w * 4 + 4].copy_from_slice(&bytes);
                            }
                        }

                        // 524 (0x20C): vspltb (Vector Splat Byte)
                        524 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let uimm = ((insn >> 16) & 0xF) as usize;
                            let b = self.regs.vr[vb][uimm];
                            self.regs.vr[vd].fill(b);
                        }

                        // 588 (0x24C): vsplth (Vector Splat Halfword)
                        588 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let uimm = (((insn >> 16) & 0x7) * 2) as usize;
                            let hw = [self.regs.vr[vb][uimm], self.regs.vr[vb][uimm + 1]];
                            for h in 0..8 {
                                self.regs.vr[vd][h * 2..h * 2 + 2].copy_from_slice(&hw);
                            }
                        }

                        // 652 (0x28C): vspltw (Vector Splat Word)
                        652 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let uimm = (((insn >> 16) & 0x3) * 4) as usize;
                            let word: [u8; 4] = [
                                self.regs.vr[vb][uimm],
                                self.regs.vr[vb][uimm + 1],
                                self.regs.vr[vb][uimm + 2],
                                self.regs.vr[vb][uimm + 3],
                            ];
                            for w in 0..4 {
                                self.regs.vr[vd][w * 4..w * 4 + 4].copy_from_slice(&word);
                            }
                        }

                        // 140 (0x08C): vmrghw (Vector Merge High Word)
                        140 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let mut res = [0u8; 16];
                            res[0..4].copy_from_slice(&self.regs.vr[va][0..4]);
                            res[4..8].copy_from_slice(&self.regs.vr[vb][0..4]);
                            res[8..12].copy_from_slice(&self.regs.vr[va][4..8]);
                            res[12..16].copy_from_slice(&self.regs.vr[vb][4..8]);
                            self.regs.vr[vd] = res;
                        }

                        // 396 (0x18C): vmrglw (Vector Merge Low Word)
                        396 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let mut res = [0u8; 16];
                            res[0..4].copy_from_slice(&self.regs.vr[va][8..12]);
                            res[4..8].copy_from_slice(&self.regs.vr[vb][8..12]);
                            res[8..12].copy_from_slice(&self.regs.vr[va][12..16]);
                            res[12..16].copy_from_slice(&self.regs.vr[vb][12..16]);
                            self.regs.vr[vd] = res;
                        }

                        // 128 (0x080): vadduwm (Vector Add Unsigned Word Modulo)
                        128 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            for w in 0..4 {
                                let a = u32::from_be_bytes(self.regs.vr[va][w*4..w*4+4].try_into().unwrap());
                                let b = u32::from_be_bytes(self.regs.vr[vb][w*4..w*4+4].try_into().unwrap());
                                let sum = a.wrapping_add(b);
                                self.regs.vr[vd][w*4..w*4+4].copy_from_slice(&sum.to_be_bytes());
                            }
                        }

                        // 1204 | 1220: vxor (Vector XOR)
                        1204 | 1220 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            for i in 0..16 {
                                self.regs.vr[vd][i] = self.regs.vr[va][i] ^ self.regs.vr[vb][i];
                            }
                        }

                        // 1140 | 1156: vor / vmr (Vector OR)
                        1140 | 1156 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            for i in 0..16 {
                                self.regs.vr[vd][i] = self.regs.vr[va][i] | self.regs.vr[vb][i];
                            }
                        }

                        // 1028 (0x404): vand (Vector AND)
                        1028 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            for i in 0..16 {
                                self.regs.vr[vd][i] = self.regs.vr[va][i] & self.regs.vr[vb][i];
                            }
                        }

                        // 1092 (0x444): vandc (Vector AND with Complement)
                        1092 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            for i in 0..16 {
                                self.regs.vr[vd][i] = self.regs.vr[va][i] & (!self.regs.vr[vb][i]);
                            }
                        }

                        // 1268 | 1284: vnor (Vector NOR)
                        1268 | 1284 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            for i in 0..16 {
                                self.regs.vr[vd][i] = !(self.regs.vr[va][i] | self.regs.vr[vb][i]);
                            }
                        }

                        // 260 (0x104): vsl (Vector Shift Left)
                        260 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let sh = (self.regs.vr[vb][15] & 7) as u32;
                            if sh == 0 {
                                self.regs.vr[vd] = self.regs.vr[va];
                            } else {
                                let mut carry = 0u8;
                                let mut res = [0u8; 16];
                                for i in (0..16).rev() {
                                    let b = self.regs.vr[va][i];
                                    res[i] = (b << sh) | carry;
                                    carry = b >> (8 - sh);
                                }
                                self.regs.vr[vd] = res;
                            }
                        }

                        // 516 (0x204): vsr (Vector Shift Right)
                        516 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let sh = (self.regs.vr[vb][15] & 7) as u32;
                            if sh == 0 {
                                self.regs.vr[vd] = self.regs.vr[va];
                            } else {
                                let mut carry = 0u8;
                                let mut res = [0u8; 16];
                                for i in 0..16 {
                                    let b = self.regs.vr[va][i];
                                    res[i] = (b >> sh) | carry;
                                    carry = b << (8 - sh);
                                }
                                self.regs.vr[vd] = res;
                            }
                        }

                        // 388 (0x184): vslo (Vector Shift Left by Octet)
                        388 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let sh = ((self.regs.vr[vb][15] >> 3) & 0xF) as usize;
                            let mut res = [0u8; 16];
                            for i in 0..16 {
                                if i + sh < 16 {
                                    res[i] = self.regs.vr[va][i + sh];
                                }
                            }
                            self.regs.vr[vd] = res;
                        }

                        // 644 (0x284): vsro (Vector Shift Right by Octet)
                        644 => {
                            let vd = ((insn >> 21) & 0x1F) as usize;
                            let va = ((insn >> 16) & 0x1F) as usize;
                            let vb = ((insn >> 11) & 0x1F) as usize;
                            let sh = ((self.regs.vr[vb][15] >> 3) & 0xF) as usize;
                            let mut res = [0u8; 16];
                            for i in 0..16 {
                                if i >= sh {
                                    res[i] = self.regs.vr[va][i - sh];
                                }
                            }
                            self.regs.vr[vd] = res;
                        }

                        _ => return Err(format!("Unimplemented AltiVec opcode 4 xo {} (0x{:03X}) at PC=0x{:08X}", xo, xo, pc)),
                    }
                }
            }

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

            // Opcode 12: addic (Add Immediate Carrying)
            12 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i32 as u32;
                let a = self.regs.gpr[ra] as u32;
                let sum = (a as u64).wrapping_add(simm as u64);
                let ca = if sum > 0xFFFF_FFFF { 1 } else { 0 };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[rd] = (sum as u32) as u64;
            }

            // Opcode 13: addic. (Add Immediate Carrying and Record)
            13 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let simm = (insn & 0xFFFF) as i16 as i32 as u32;
                let a = self.regs.gpr[ra] as u32;
                let sum = (a as u64).wrapping_add(simm as u64);
                let ca = if sum > 0xFFFF_FFFF { 1 } else { 0 };
                self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                self.regs.gpr[rd] = (sum as u32) as u64;
                self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
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
                let rc = insn & 1;

                let val = (self.regs.gpr[rs] as u32).rotate_left(sh);
                let mask = ppc_mask32(mb, me);
                let res = val & mask;
                self.regs.gpr[ra] = res as u64;
                if rc != 0 {
                    self.regs.update_cr_cmpw(0, res as i32, 0);
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
                    self.regs.update_cr_cmpw(0, res as i32, 0);
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

                    // 20: lwarx (Load Word and Reserve Indexed)
                    20 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = bus.read_u32(ea)?;
                        self.regs.gpr[rd] = val as u64;
                    }

                    // 84: ldarx (Load Doubleword and Reserve Indexed)
                    84 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = bus.read_u64(ea)?;
                        self.regs.gpr[rd] = val;
                    }

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

                    // 150: stwcx. (Store Word Conditional Indexed)
                    150 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        bus.write_u32(ea, self.regs.gpr[rs] as u32)?;
                        let cr0_shift = 7 * 4;
                        self.regs.cr = (self.regs.cr & !(0xFu32 << cr0_shift)) | (0b0010 << cr0_shift);
                    }

                    // 214: stdcx. (Store Doubleword Conditional Indexed)
                    214 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        bus.write_u64(ea, self.regs.gpr[rs])?;
                        let cr0_shift = 7 * 4;
                        self.regs.cr = (self.regs.cr & !(0xFu32 << cr0_shift)) | (0b0010 << cr0_shift);
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

                    // add / add.
                    266 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let res = self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]);
                        self.regs.gpr[rd] = res;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, res as i32, 0);
                        }
                    }

                    // 10: addc (Add Carrying)
                    10 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let a = self.regs.gpr[ra] as u32;
                        let b = self.regs.gpr[rb] as u32;
                        let sum = (a as u64).wrapping_add(b as u64);
                        let ca = if sum > 0xFFFF_FFFF { 1 } else { 0 };
                        self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                        self.regs.gpr[rd] = (sum as u32) as u64;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                        }
                    }

                    // 138: adde (Add Extended)
                    138 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ca = (self.regs.xer >> 29) & 1;
                        let a = self.regs.gpr[ra] as u32;
                        let b = self.regs.gpr[rb] as u32;
                        let sum = (a as u64).wrapping_add(b as u64).wrapping_add(ca);
                        let new_ca = if sum > 0xFFFF_FFFF { 1 } else { 0 };
                        self.regs.xer = (self.regs.xer & !(1 << 29)) | (new_ca << 29);
                        self.regs.gpr[rd] = (sum as u32) as u64;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                        }
                    }

                    // 202: addze (Add to Zero Extended)
                    202 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let ca = (self.regs.xer >> 29) & 1;
                        let a = self.regs.gpr[ra] as u32;
                        let sum = (a as u64).wrapping_add(ca);
                        let new_ca = if sum > 0xFFFF_FFFF { 1 } else { 0 };
                        self.regs.xer = (self.regs.xer & !(1 << 29)) | (new_ca << 29);
                        self.regs.gpr[rd] = (sum as u32) as u64;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                        }
                    }

                    // 8: subfc (Subtract From Carrying)
                    8 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let a = self.regs.gpr[ra] as u32;
                        let b = self.regs.gpr[rb] as u32;
                        let diff = (!a as u64).wrapping_add(b as u64).wrapping_add(1);
                        let ca = if diff > 0xFFFF_FFFF { 1 } else { 0 };
                        self.regs.xer = (self.regs.xer & !(1 << 29)) | (ca << 29);
                        self.regs.gpr[rd] = (diff as u32) as u64;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                        }
                    }

                    // 136: subfe (Subtract From Extended)
                    136 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ca = (self.regs.xer >> 29) & 1;
                        let a = self.regs.gpr[ra] as u32;
                        let b = self.regs.gpr[rb] as u32;
                        let diff = (!a as u64).wrapping_add(b as u64).wrapping_add(ca);
                        let new_ca = if diff > 0xFFFF_FFFF { 1 } else { 0 };
                        self.regs.xer = (self.regs.xer & !(1 << 29)) | (new_ca << 29);
                        self.regs.gpr[rd] = (diff as u32) as u64;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, self.regs.gpr[rd] as i32, 0);
                        }
                    }

                    // 104: neg (Negate)
                    104 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let res = (0u64).wrapping_sub(self.regs.gpr[ra]);
                        self.regs.gpr[rd] = res;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, res as i32, 0);
                        }
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
                        self.regs.gpr[ra] = res;
                        if (insn & 1) != 0 {
                            self.regs.update_cr_cmpw(0, res as i32, 0);
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

                    // 23: lwzx (Load Word and Zero Indexed)
                    23 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = bus.read_u32(ea)?;
                        self.regs.gpr[rd] = val as u64;
                    }

                    // 87: lbzx (Load Byte and Zero Indexed)
                    87 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = bus.read_u8(ea)?;
                        self.regs.gpr[rd] = val as u64;
                    }

                    // 279: lhzx (Load Halfword and Zero Indexed)
                    279 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = bus.read_u16(ea)?;
                        self.regs.gpr[rd] = val as u64;
                    }

                    // 21: ldx (Load Doubleword Indexed)
                    21 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = bus.read_u64(ea)?;
                        self.regs.gpr[rd] = val;
                    }

                    // 151: stwx (Store Word Indexed)
                    151 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        bus.write_u32(ea, self.regs.gpr[rs] as u32)?;
                    }

                    // 215: stbx (Store Byte Indexed)
                    215 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        bus.write_u8(ea, self.regs.gpr[rs] as u8)?;
                    }

                    // 407: sthx (Store Halfword Indexed)
                    407 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        bus.write_u16(ea, self.regs.gpr[rs] as u16)?;
                    }

                    // 149: stdx (Store Doubleword Indexed)
                    149 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        bus.write_u64(ea, self.regs.gpr[rs])?;
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

                    // 535: lfsx (Load Floating-Point Single Indexed)
                    535 => {
                        let frd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let bits = bus.read_u32(ea)?;
                        self.regs.fpr[frd] = f32::from_bits(bits) as f64;
                    }

                    // 599: lfdx (Load Floating-Point Double Indexed)
                    599 => {
                        let frd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let bits = bus.read_u64(ea)?;
                        self.regs.fpr[frd] = f64::from_bits(bits);
                    }

                    // 663: stfsx (Store Floating-Point Single Indexed)
                    663 => {
                        let frs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = (self.regs.fpr[frs] as f32).to_bits();
                        bus.write_u32(ea, val)?;
                    }

                    // 727: stfdx (Store Floating-Point Double Indexed)
                    727 => {
                        let frs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = self.regs.fpr[frs].to_bits();
                        bus.write_u64(ea, val)?;
                    }

                    // 38: lvsl (Load Vector for Shift Left)
                    38 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let sh = (ea & 0xF) as u8;
                        for i in 0..16 {
                            self.regs.vr[vd][i] = (i as u8).wrapping_add(sh);
                        }
                    }

                    // 102: lvsr (Load Vector for Shift Right)
                    102 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let sh = (ea & 0xF) as u8;
                        for i in 0..16 {
                            self.regs.vr[vd][i] = (i as u8).wrapping_add(16 - sh);
                        }
                    }

                    // 103 | 359: lvx / lvxl (Load Vector Indexed)
                    103 | 359 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = (if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) }) & !0xF;
                        for i in 0..16 {
                            self.regs.vr[vd][i] = bus.read_u8(ea + i as u64)?;
                        }
                    }

                    // 231 | 487: stvx / stvxl (Store Vector Indexed)
                    231 | 487 => {
                        let vs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = (if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) }) & !0xF;
                        for i in 0..16 {
                            bus.write_u8(ea + i as u64, self.regs.vr[vs][i])?;
                        }
                    }

                    // 3 | 7: lvebx (Load Vector Element Byte Indexed)
                    3 | 7 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let idx = (ea & 0xF) as usize;
                        self.regs.vr[vd][idx] = bus.read_u8(ea)?;
                    }

                    // 19 | 39: lvehx (Load Vector Element Halfword Indexed)
                    19 | 39 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let idx = (ea & 0xE) as usize;
                        let val = bus.read_u16(ea & !1)?;
                        let bytes = val.to_be_bytes();
                        self.regs.vr[vd][idx] = bytes[0];
                        self.regs.vr[vd][idx + 1] = bytes[1];
                    }

                    // 6 | 71: lvewx (Load Vector Element Word Indexed)
                    6 | 71 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let idx = (ea & 0xC) as usize;
                        let val = bus.read_u32(ea & !3)?;
                        let bytes = val.to_be_bytes();
                        self.regs.vr[vd][idx..idx + 4].copy_from_slice(&bytes);
                    }

                    // 67 | 135: stvebx (Store Vector Element Byte Indexed)
                    67 | 135 => {
                        let vs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let idx = (ea & 0xF) as usize;
                        bus.write_u8(ea, self.regs.vr[vs][idx])?;
                    }

                    // 167: stvehx (Store Vector Element Halfword Indexed)
                    167 => {
                        let vs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let idx = (ea & 0xE) as usize;
                        let val = u16::from_be_bytes([self.regs.vr[vs][idx], self.regs.vr[vs][idx + 1]]);
                        bus.write_u16(ea & !1, val)?;
                    }

                    // 70 | 134 | 199: stvewx (Store Vector Element Word Indexed)
                    70 | 134 | 199 => {
                        let vs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let idx = (ea & 0xC) as usize;
                        let mut b = [0u8; 4];
                        b.copy_from_slice(&self.regs.vr[vs][idx..idx + 4]);
                        let val = u32::from_be_bytes(b);
                        bus.write_u32(ea & !3, val)?;
                    }

                    // 519 | 775: lvlx / lvlxl (Load Vector Left Indexed)
                    519 | 775 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let eb = (ea & 0xF) as usize;
                        let aligned_ea = ea & !0xF;
                        let mut res = [0u8; 16];
                        for i in 0..(16 - eb) {
                            res[i] = bus.read_u8(aligned_ea + (eb + i) as u64)?;
                        }
                        self.regs.vr[vd] = res;
                    }

                    // 551 | 807: lvrx / lvrxl (Load Vector Right Indexed)
                    551 | 807 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let eb = (ea & 0xF) as usize;
                        let mut res = [0u8; 16];
                        if eb != 0 {
                            let aligned_ea = ea & !0xF;
                            for i in 0..eb {
                                res[16 - eb + i] = bus.read_u8(aligned_ea + i as u64)?;
                            }
                        }
                        self.regs.vr[vd] = res;
                    }

                    // 647 | 903: stvlx / stvlxl (Store Vector Left Indexed)
                    647 | 903 => {
                        let vs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let eb = (ea & 0xF) as usize;
                        let aligned_ea = ea & !0xF;
                        for i in 0..(16 - eb) {
                            bus.write_u8(aligned_ea + (eb + i) as u64, self.regs.vr[vs][i])?;
                        }
                    }

                    // 679 | 935: stvrx / stvrxl (Store Vector Right Indexed)
                    679 | 935 => {
                        let vs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let eb = (ea & 0xF) as usize;
                        if eb != 0 {
                            let aligned_ea = ea & !0xF;
                            for i in 0..eb {
                                bus.write_u8(aligned_ea + i as u64, self.regs.vr[vs][16 - eb + i])?;
                            }
                        }
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

            // Opcode 33: lwzu (Load Word and Zero with Update)
            33 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let val = bus.read_u32(ea)?;
                self.regs.gpr[rd] = val as u64;
                self.regs.gpr[ra] = ea;
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

            // Opcode 35: lbzu (Load Byte and Zero with Update)
            35 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let val = bus.read_u8(ea)?;
                self.regs.gpr[rd] = val as u64;
                self.regs.gpr[ra] = ea;
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

            // Opcode 39: stbu (Store Byte with Update)
            39 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                bus.write_u8(ea, self.regs.gpr[rs] as u8)?;
                self.regs.gpr[ra] = ea;
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

            // Opcode 41: lhzu (Load Halfword and Zero with Update)
            41 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let val = bus.read_u16(ea)?;
                self.regs.gpr[rd] = val as u64;
                self.regs.gpr[ra] = ea;
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

            // Opcode 43: lhau (Load Halfword Algebraic with Update)
            43 => {
                let rd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let val = bus.read_u16(ea)? as i16 as i64 as u64;
                self.regs.gpr[rd] = val;
                self.regs.gpr[ra] = ea;
            }

            // Opcode 44: sth (Store Halfword)
            44 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                bus.write_u16(ea, self.regs.gpr[rs] as u16)?;
            }

            // Opcode 45: sthu (Store Halfword with Update)
            45 => {
                let rs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                bus.write_u16(ea, self.regs.gpr[rs] as u16)?;
                self.regs.gpr[ra] = ea;
            }

            // Opcode 48: lfs (Load Floating-Point Single)
            48 => {
                let frd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let bits = bus.read_u32(ea)?;
                let f = f32::from_bits(bits) as f64;
                self.regs.fpr[frd] = f;
            }

            // Opcode 49: lfsu (Load Floating-Point Single with Update)
            49 => {
                let frd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let bits = bus.read_u32(ea)?;
                let f = f32::from_bits(bits) as f64;
                self.regs.fpr[frd] = f;
                self.regs.gpr[ra] = ea;
            }

            // Opcode 50: lfd (Load Floating-Point Double)
            50 => {
                let frd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let bits = bus.read_u64(ea)?;
                self.regs.fpr[frd] = f64::from_bits(bits);
            }

            // Opcode 51: lfdu (Load Floating-Point Double with Update)
            51 => {
                let frd = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let bits = bus.read_u64(ea)?;
                self.regs.fpr[frd] = f64::from_bits(bits);
                self.regs.gpr[ra] = ea;
            }

            // Opcode 52: stfs (Store Floating-Point Single)
            52 => {
                let frs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let val = (self.regs.fpr[frs] as f32).to_bits();
                bus.write_u32(ea, val)?;
            }

            // Opcode 53: stfsu (Store Floating-Point Single with Update)
            53 => {
                let frs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let val = (self.regs.fpr[frs] as f32).to_bits();
                bus.write_u32(ea, val)?;
                self.regs.gpr[ra] = ea;
            }

            // Opcode 54: stfd (Store Floating-Point Double)
            54 => {
                let frs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = if ra == 0 { d } else { self.regs.gpr[ra].wrapping_add(d) };
                let val = self.regs.fpr[frs].to_bits();
                bus.write_u64(ea, val)?;
            }

            // Opcode 55: stfdu (Store Floating-Point Double with Update)
            55 => {
                let frs = ((insn >> 21) & 0x1F) as usize;
                let ra = ((insn >> 16) & 0x1F) as usize;
                let d = (insn & 0xFFFF) as i16 as i64 as u64;
                let ea = self.regs.gpr[ra].wrapping_add(d);
                let val = self.regs.fpr[frs].to_bits();
                bus.write_u64(ea, val)?;
                self.regs.gpr[ra] = ea;
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

            // Opcode 59: Single-Precision Floating-Point Operations
            59 => {
                let frd = ((insn >> 21) & 0x1F) as usize;
                let fra = ((insn >> 16) & 0x1F) as usize;
                let frb = ((insn >> 11) & 0x1F) as usize;
                let frc = ((insn >> 6) & 0x1F) as usize;
                let xo = (insn >> 1) & 0x1F;
                match xo {
                    // 18: fdivs
                    18 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        self.regs.fpr[frd] = (a / b) as f64;
                    }
                    // 20: fsubs
                    20 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        self.regs.fpr[frd] = (a - b) as f64;
                    }
                    // 21: fadds
                    21 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        self.regs.fpr[frd] = (a + b) as f64;
                    }
                    // 22: fsqrts
                    22 => {
                        let b = self.regs.fpr[frb] as f32;
                        self.regs.fpr[frd] = b.sqrt() as f64;
                    }
                    // 24: fres
                    24 => {
                        let b = self.regs.fpr[frb] as f32;
                        self.regs.fpr[frd] = (1.0f32 / b) as f64;
                    }
                    // 25: fmuls
                    25 => {
                        let a = self.regs.fpr[fra] as f32;
                        let c = self.regs.fpr[frc] as f32;
                        self.regs.fpr[frd] = (a * c) as f64;
                    }
                    // 28: fmsubs
                    28 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        let c = self.regs.fpr[frc] as f32;
                        self.regs.fpr[frd] = ((a * c) - b) as f64;
                    }
                    // 29: fmadds
                    29 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        let c = self.regs.fpr[frc] as f32;
                        self.regs.fpr[frd] = ((a * c) + b) as f64;
                    }
                    // 30: fnmsubs
                    30 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        let c = self.regs.fpr[frc] as f32;
                        self.regs.fpr[frd] = -(((a * c) - b)) as f64;
                    }
                    // 31: fnmadds
                    31 => {
                        let a = self.regs.fpr[fra] as f32;
                        let b = self.regs.fpr[frb] as f32;
                        let c = self.regs.fpr[frc] as f32;
                        self.regs.fpr[frd] = -(((a * c) + b)) as f64;
                    }
                    _ => return Err(format!("Unimplemented opcode 59 xo {} at PC=0x{:08X}", xo, pc)),
                }
            }

            // Opcode 63: Double-Precision Floating-Point Operations
            63 => {
                let frd = ((insn >> 21) & 0x1F) as usize;
                let fra = ((insn >> 16) & 0x1F) as usize;
                let frb = ((insn >> 11) & 0x1F) as usize;
                let frc = ((insn >> 6) & 0x1F) as usize;
                let xo5 = (insn >> 1) & 0x1F;
                let xo10 = (insn >> 1) & 0x3FF;

                // Check A-form arithmetic operations first
                match xo5 {
                    18 => { self.regs.fpr[frd] = self.regs.fpr[fra] / self.regs.fpr[frb]; }
                    20 => { self.regs.fpr[frd] = self.regs.fpr[fra] - self.regs.fpr[frb]; }
                    21 => { self.regs.fpr[frd] = self.regs.fpr[fra] + self.regs.fpr[frb]; }
                    22 => { self.regs.fpr[frd] = self.regs.fpr[frb].sqrt(); }
                    23 => {
                        // fsel: frd = if fra >= 0.0 { frc } else { frb }
                        self.regs.fpr[frd] = if self.regs.fpr[fra] >= 0.0 { self.regs.fpr[frc] } else { self.regs.fpr[frb] };
                    }
                    25 => { self.regs.fpr[frd] = self.regs.fpr[fra] * self.regs.fpr[frc]; }
                    26 => { self.regs.fpr[frd] = 1.0 / self.regs.fpr[frb].sqrt(); }
                    28 => { self.regs.fpr[frd] = (self.regs.fpr[fra] * self.regs.fpr[frc]) - self.regs.fpr[frb]; }
                    29 => { self.regs.fpr[frd] = (self.regs.fpr[fra] * self.regs.fpr[frc]) + self.regs.fpr[frb]; }
                    30 => { self.regs.fpr[frd] = -((self.regs.fpr[fra] * self.regs.fpr[frc]) - self.regs.fpr[frb]); }
                    31 => { self.regs.fpr[frd] = -((self.regs.fpr[fra] * self.regs.fpr[frc]) + self.regs.fpr[frb]); }
                    _ => {
                        // X-form operations
                        match xo10 {
                            0 => {
                                // fcmpu
                                let crf = ((insn >> 23) & 0x7) as u32;
                                self.regs.update_cr_cmpf(crf, self.regs.fpr[fra], self.regs.fpr[frb]);
                            }
                            32 => {
                                // fcmpo
                                let crf = ((insn >> 23) & 0x7) as u32;
                                self.regs.update_cr_cmpf(crf, self.regs.fpr[fra], self.regs.fpr[frb]);
                            }
                            12 => {
                                // frsp: Round to Single Precision
                                self.regs.fpr[frd] = (self.regs.fpr[frb] as f32) as f64;
                            }
                            14 => {
                                // fctiw: Convert to Integer Word
                                self.regs.fpr[frd] = f64::from_bits(self.regs.fpr[frb] as i32 as u64);
                            }
                            15 => {
                                // fctiwz: Convert to Integer Word with Round towards Zero
                                self.regs.fpr[frd] = f64::from_bits(self.regs.fpr[frb].trunc() as i32 as u64);
                            }
                            40 => {
                                // fneg
                                self.regs.fpr[frd] = -self.regs.fpr[frb];
                            }
                            72 => {
                                // fmr
                                self.regs.fpr[frd] = self.regs.fpr[frb];
                            }
                            136 => {
                                // fnabs
                                self.regs.fpr[frd] = -self.regs.fpr[frb].abs();
                            }
                            264 => {
                                // fabs
                                self.regs.fpr[frd] = self.regs.fpr[frb].abs();
                            }
                            814 => {
                                // fctid: Convert to Integer Doubleword
                                self.regs.fpr[frd] = f64::from_bits(self.regs.fpr[frb] as i64 as u64);
                            }
                            815 => {
                                // fctidz: Convert to Integer Doubleword with Round towards Zero
                                self.regs.fpr[frd] = f64::from_bits(self.regs.fpr[frb].trunc() as i64 as u64);
                            }
                            846 => {
                                // fcfid: Convert from Integer Doubleword
                                self.regs.fpr[frd] = (self.regs.fpr[frb].to_bits() as i64) as f64;
                            }
                            583 => {
                                // mffs: Move from FPSCR
                                self.regs.fpr[frd] = f64::from_bits(self.regs.fpscr);
                            }
                            711 => {
                                // mtfsf: Move to FPSCR Fields
                                self.regs.fpscr = self.regs.fpr[frb].to_bits();
                            }
                            _ => return Err(format!("Unimplemented opcode 63 xo10 {} (xo5 {}) at PC=0x{:08X}", xo10, xo5, pc)),
                        }
                    }
                }
            }

            _ => return Err(format!("Unimplemented PowerPC opcode: {} (0x{:02X}) at PC=0x{:016X}", opcode, opcode, pc)),
        }

        Ok(StepResult::Ok)
    }
}
