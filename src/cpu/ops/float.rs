//! Floating-point arithmetic (primary opcodes 59 and 63).

use crate::cpu::interpreter::{Cpu, StepResult};
use crate::memory::MemoryBus;

impl Cpu {
    /// Primary opcodes: 59 | 63.
    #[inline(always)]
    pub(in crate::cpu) fn exec_float(&mut self, _bus: &mut MemoryBus, insn: u32, pc: u64, opcode: u32) -> Result<StepResult, String> {
        match opcode {

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

            _ => unreachable!("exec_float: opcode {} not in this group", opcode),
        }

        Ok(StepResult::Ok)
    }
}
