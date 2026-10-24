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
                // AltiVec VA-form: 6-bit xo in bits 0..5, valid range 32..47 (VX forms never use these)
                let va_xo = insn & 0x3F;
                if (32..=47).contains(&va_xo) {
                    let vd = ((insn >> 21) & 0x1F) as usize;
                    let va = ((insn >> 16) & 0x1F) as usize;
                    let vb = ((insn >> 11) & 0x1F) as usize;
                    let vc = ((insn >> 6) & 0x1F) as usize;
                    let a = self.regs.vr[va];
                    let b = self.regs.vr[vb];
                    let c = self.regs.vr[vc];
                    let mut res = [0u8; 16];
                    match va_xo {
                        // vperm
                        43 => {
                            for i in 0..16 {
                                let sel = (c[i] & 0x1F) as usize;
                                res[i] = if sel < 16 { a[sel] } else { b[sel - 16] };
                            }
                        }
                        // vsel: (a & ~c) | (b & c)
                        42 => for i in 0..16 { res[i] = (a[i] & !c[i]) | (b[i] & c[i]); },
                        // vsldoi: shift left double by octet immediate (SHB = bits 6..9)
                        44 => {
                            let sh = ((insn >> 6) & 0xF) as usize;
                            let mut cat = [0u8; 32];
                            cat[..16].copy_from_slice(&a);
                            cat[16..].copy_from_slice(&b);
                            res.copy_from_slice(&cat[sh..sh + 16]);
                        }
                        // vmaddfp: a*c + b ; vnmsubfp: -(a*c - b)
                        46 => for i in 0..4 { set_vf(&mut res, i, vf(&a, i) * vf(&c, i) + vf(&b, i)); },
                        47 => for i in 0..4 { set_vf(&mut res, i, -(vf(&a, i) * vf(&c, i) - vf(&b, i))); },
                        // vmladduhm: low 16 bits of a*b + c per halfword
                        34 => {
                            for i in 0..8 {
                                let x = u16::from_be_bytes([a[i * 2], a[i * 2 + 1]]);
                                let y = u16::from_be_bytes([b[i * 2], b[i * 2 + 1]]);
                                let z = u16::from_be_bytes([c[i * 2], c[i * 2 + 1]]);
                                res[i * 2..i * 2 + 2].copy_from_slice(&x.wrapping_mul(y).wrapping_add(z).to_be_bytes());
                            }
                        }
                        // vmhaddshs / vmhraddshs: ((a*b [+0x4000]) >> 15) + c, saturated signed halfword
                        32 | 33 => {
                            for i in 0..8 {
                                let x = i16::from_be_bytes([a[i * 2], a[i * 2 + 1]]) as i32;
                                let y = i16::from_be_bytes([b[i * 2], b[i * 2 + 1]]) as i32;
                                let z = i16::from_be_bytes([c[i * 2], c[i * 2 + 1]]) as i32;
                                let p = x * y + if va_xo == 33 { 0x4000 } else { 0 };
                                let r = ((p >> 15) + z).clamp(-32768, 32767) as i16;
                                res[i * 2..i * 2 + 2].copy_from_slice(&r.to_be_bytes());
                            }
                        }
                        // vmsumubm (36) / vmsummbm (37): byte products summed in groups of 4 into words + c
                        36 | 37 => {
                            for i in 0..4 {
                                let mut sum = u32::from_be_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]]);
                                for j in 0..4 {
                                    let x = a[i * 4 + j];
                                    let y = b[i * 4 + j];
                                    let p: i64 = if va_xo == 36 { x as i64 * y as i64 } else { (x as i8 as i64) * y as i64 };
                                    sum = sum.wrapping_add(p as u32);
                                }
                                res[i * 4..i * 4 + 4].copy_from_slice(&sum.to_be_bytes());
                            }
                        }
                        // vmsumuhm (38) / vmsumuhs (39) / vmsumshm (40) / vmsumshs (41): halfword products summed in pairs + c
                        _ => {
                            for i in 0..4 {
                                let mut sum: i64 = u32::from_be_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]]) as i64;
                                let signed = va_xo >= 40;
                                if signed { sum = i32::from_be_bytes([c[i * 4], c[i * 4 + 1], c[i * 4 + 2], c[i * 4 + 3]]) as i64; }
                                for j in 0..2 {
                                    let o = i * 4 + j * 2;
                                    let (xv, yv) = if signed {
                                        (i16::from_be_bytes([a[o], a[o + 1]]) as i64, i16::from_be_bytes([b[o], b[o + 1]]) as i64)
                                    } else {
                                        (u16::from_be_bytes([a[o], a[o + 1]]) as i64, u16::from_be_bytes([b[o], b[o + 1]]) as i64)
                                    };
                                    sum += xv * yv;
                                }
                                let w: u32 = match va_xo {
                                    38 | 40 => sum as u32,
                                    39 => sum.clamp(0, u32::MAX as i64) as u32,
                                    _ => sum.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32,
                                };
                                res[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
                            }
                        }
                    }
                    self.regs.vr[vd] = res;
                } else if (insn & 0x10) != 0 && (insn & 3) != 3 || ((insn & 0x10) != 0 && !matches!((insn >> 4) & 0x7F, 0 | 4 | 8 | 12 | 24 | 28 | 44 | 60 | 64 | 68 | 80 | 84 | 96 | 100 | 112 | 116)) {
                    // vsldoi128: |000100|VD128|VA128|VB128|A|SHB|a|1|VDh|VBh|
                    let vd = ((((insn >> 2) & 0x3) << 5) | ((insn >> 21) & 0x1F)) as usize;
                    let vb = (((insn & 0x3) << 5) | ((insn >> 11) & 0x1F)) as usize;
                    let va = ((((insn >> 10) & 1) << 6) | (((insn >> 5) & 1) << 5) | ((insn >> 16) & 0x1F)) as usize;
                    let sh = ((insn >> 6) & 0xF) as usize;
                    let mut cat = [0u8; 32];
                    cat[..16].copy_from_slice(&self.regs.vr[va]);
                    cat[16..].copy_from_slice(&self.regs.vr[vb]);
                    self.regs.vr[vd].copy_from_slice(&cat[sh..sh + 16]);
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

                        // 0 / 4: lvsl128 / lvsr128 (Load Vector for Shift Left / Right)
                        0 | 4 => {
                            let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                            let sh = (ea & 0xF) as u8;
                            for i in 0..16u8 {
                                self.regs.vr[vr][i as usize] = if op7 == 0 { i.wrapping_add(sh) } else { i.wrapping_add(16 - sh) };
                            }
                        }

                        // 24: stvewx128 (Store Vector Element Word Indexed). NOTE: real code uses op7=24 (xenia vmx128.txt lists 48, which is wrong)
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
                    crate::cpu::altivec::exec_vx(&mut self.regs, insn).map_err(|e| format!("{} at PC=0x{:08X}", e, pc))?;
                }
            }

            // Opcode 5: VMX128 arithmetic / logical / pack / permute (VX128 form, see xenia docs/ppc/vmx128.txt)
            // Layout (LSB numbering): VD128[25:21] VA128[20:16] VB128[15:11] A[10] xo4[9:6] a[5] flag[4] VDh[3:2] VBh[1:0]
            5 => {
                let vd = ((((insn >> 2) & 0x3) << 5) | ((insn >> 21) & 0x1F)) as usize;
                let vb = (((insn & 0x3) << 5) | ((insn >> 11) & 0x1F)) as usize;
                let va = ((((insn >> 10) & 1) << 6) | (((insn >> 5) & 1) << 5) | ((insn >> 16) & 0x1F)) as usize;
                let x4 = (insn >> 6) & 0xF;
                let bit4 = (insn & 0x10) != 0;

                if !bit4 && (insn & 0x200) == 0 {
                    // vperm128: VC is a 3-bit register number (vr0..vr7) in bits 8..6
                    let vc = ((insn >> 6) & 0x7) as usize;
                    let mut res = [0u8; 16];
                    for i in 0..16 {
                        let sel = (self.regs.vr[vc][i] & 0x1F) as usize;
                        res[i] = if sel < 16 { self.regs.vr[va][sel] } else { self.regs.vr[vb][sel - 16] };
                    }
                    self.regs.vr[vd] = res;
                } else if !bit4 {
                    // Pack instructions (xo4 = 1000..1111)
                    let a = self.regs.vr[va];
                    let b = self.regs.vr[vb];
                    let mut res = [0u8; 16];
                    match x4 {
                        // vpkshss128 / vpkshus128 / vpkuhum128 / vpkuhus128: halfwords -> bytes
                        0b1000 | 0b1001 | 0b1100 | 0b1101 => {
                            for i in 0..16 {
                                let src = if i < 8 { &a } else { &b };
                                let j = (i % 8) * 2;
                                let hw = u16::from_be_bytes([src[j], src[j + 1]]);
                                res[i] = match x4 {
                                    0b1000 => (hw as i16).clamp(-128, 127) as i8 as u8,
                                    0b1001 => (hw as i16).clamp(0, 255) as u8,
                                    0b1100 => hw as u8,
                                    _ => hw.min(255) as u8,
                                };
                            }
                        }
                        // vpkswss128 / vpkswus128 / vpkuwum128 / vpkuwus128: words -> halfwords
                        _ => {
                            for i in 0..8 {
                                let src = if i < 4 { &a } else { &b };
                                let j = (i % 4) * 4;
                                let w = u32::from_be_bytes([src[j], src[j + 1], src[j + 2], src[j + 3]]);
                                let h: u16 = match x4 {
                                    0b1010 => (w as i32).clamp(-32768, 32767) as i16 as u16,
                                    0b1011 => (w as i32).clamp(0, 65535) as u16,
                                    0b1110 => w as u16,
                                    _ => w.min(65535) as u16,
                                };
                                res[i * 2..i * 2 + 2].copy_from_slice(&h.to_be_bytes());
                            }
                        }
                    }
                    self.regs.vr[vd] = res;
                } else {
                    let a = self.regs.vr[va];
                    let b = self.regs.vr[vb];
                    let d_old = self.regs.vr[vd];
                    let mut res = [0u8; 16];
                    match x4 {
                        // vaddfp128 / vsubfp128 / vmulfp128
                        0b0000 | 0b0001 | 0b0010 => {
                            for i in 0..4 {
                                let (x, y) = (vf(&a, i), vf(&b, i));
                                let r = match x4 { 0 => x + y, 1 => x - y, _ => x * y };
                                set_vf(&mut res, i, r);
                            }
                        }
                        // vmaddfp128: VD = VA*VB + VD
                        0b0011 => for i in 0..4 { set_vf(&mut res, i, vf(&a, i) * vf(&b, i) + vf(&d_old, i)); },
                        // vmaddcfp128: VD = VA*VD + VB
                        0b0100 => for i in 0..4 { set_vf(&mut res, i, vf(&a, i) * vf(&d_old, i) + vf(&b, i)); },
                        // vnmsubfp128: VD = -(VA*VB - VD)
                        0b0101 => for i in 0..4 { set_vf(&mut res, i, -(vf(&a, i) * vf(&b, i) - vf(&d_old, i))); },
                        // vmsum3fp128 / vmsum4fp128: dot product broadcast to all lanes
                        0b0110 | 0b0111 => {
                            let n = if x4 == 0b0110 { 3 } else { 4 };
                            let mut sum = 0.0f32;
                            for i in 0..n { sum += vf(&a, i) * vf(&b, i); }
                            for i in 0..4 { set_vf(&mut res, i, sum); }
                        }
                        // vand128 / vandc128 / vnor128 / vor128 / vxor128
                        0b1000 => for i in 0..16 { res[i] = a[i] & b[i]; },
                        0b1001 => for i in 0..16 { res[i] = a[i] & !b[i]; },
                        0b1010 => for i in 0..16 { res[i] = !(a[i] | b[i]); },
                        0b1011 => for i in 0..16 { res[i] = a[i] | b[i]; },
                        0b1100 => for i in 0..16 { res[i] = a[i] ^ b[i]; },
                        // vsel128: VD = (VA & ~VD) | (VB & VD)
                        0b1101 => for i in 0..16 { res[i] = (a[i] & !d_old[i]) | (b[i] & d_old[i]); },
                        // vslo128 / vsro128: shift left/right by octets (count in VB byte 15 bits 3..6)
                        0b1110 | 0b1111 => {
                            let sh = ((b[15] >> 3) & 0xF) as usize;
                            for i in 0..16 {
                                if x4 == 0b1110 {
                                    if i + sh < 16 { res[i] = a[i + sh]; }
                                } else if i >= sh {
                                    res[i] = a[i - sh];
                                }
                            }
                        }
                        _ => unreachable!(),
                    }
                    self.regs.vr[vd] = res;
                }
            }

            // Opcode 6: VMX128 compare / convert / merge / shift / splat / round (VX128 forms)
            6 => {
                let vd = ((((insn >> 2) & 0x3) << 5) | ((insn >> 21) & 0x1F)) as usize;
                let vb = (((insn & 0x3) << 5) | ((insn >> 11) & 0x1F)) as usize;
                let va = ((((insn >> 10) & 1) << 6) | (((insn >> 5) & 1) << 5) | ((insn >> 16) & 0x1F)) as usize;
                let imm = (insn >> 16) & 0x1F;
                let k7 = insn & 0x7F0;
                let x4 = (insn >> 6) & 0xF;
                let bit4 = (insn & 0x10) != 0;
                let b = self.regs.vr[vb];
                let a = self.regs.vr[va];
                let mut res = [0u8; 16];
                let scale = (1u64 << imm) as f32;

                match k7 {
                    // vcfpsxws128 / vcfpuxws128: float -> fixed word (saturating, toward zero)
                    0x230 => for i in 0..4 { set_vw(&mut res, i, ((vf(&b, i) * scale) as i32) as u32); },
                    0x270 => for i in 0..4 { set_vw(&mut res, i, (vf(&b, i) * scale) as u32); },
                    // vcsxwfp128 / vcuxwfp128: fixed word -> float
                    0x2B0 => for i in 0..4 { set_vf(&mut res, i, (vw(&b, i) as i32 as f32) / scale); },
                    0x2F0 => for i in 0..4 { set_vf(&mut res, i, (vw(&b, i) as f32) / scale); },
                    // vexptefp128 / vlogefp128
                    0x6B0 => for i in 0..4 { set_vf(&mut res, i, vf(&b, i).exp2()); },
                    0x6F0 => for i in 0..4 { set_vf(&mut res, i, vf(&b, i).log2()); },
                    // vrfim128 / vrfin128 / vrfip128 / vrfiz128
                    0x330 => for i in 0..4 { set_vf(&mut res, i, vf(&b, i).floor()); },
                    0x370 => for i in 0..4 { set_vf(&mut res, i, round_ties_even_f32(vf(&b, i))); },
                    0x3B0 => for i in 0..4 { set_vf(&mut res, i, vf(&b, i).ceil()); },
                    0x3F0 => for i in 0..4 { set_vf(&mut res, i, vf(&b, i).trunc()); },
                    // vrefp128 / vrsqrtefp128
                    0x630 => for i in 0..4 { set_vf(&mut res, i, 1.0 / vf(&b, i)); },
                    0x670 => for i in 0..4 { set_vf(&mut res, i, 1.0 / vf(&b, i).sqrt()); },
                    // vspltw128 / vspltisw128
                    0x730 => { let w = vw(&b, (imm & 3) as usize); for i in 0..4 { set_vw(&mut res, i, w); } }
                    0x770 => { let w = (((imm as i32) << 27) >> 27) as u32; for i in 0..4 { set_vw(&mut res, i, w); } }
                    // vupkhsb128 / vupklsb128: signed bytes -> halfwords
                    0x380 | 0x3C0 => {
                        let base = if k7 == 0x380 { 0 } else { 8 };
                        for i in 0..8 {
                            let h = (b[base + i] as i8 as i16) as u16;
                            res[i * 2..i * 2 + 2].copy_from_slice(&h.to_be_bytes());
                        }
                    }
                    // vupkd3d128: D3D-type unpack, not implemented yet
                    0x7F0 => match imm >> 2 {
                        // VPACK_D3DCOLOR: word 3 = 0xAARRGGBB -> (R, G, B, A) as 1.0f | byte
                        0 => {
                            let c = vw(&b, 3);
                            set_vw(&mut res, 0, 0x3F80_0000 | ((c >> 16) & 0xFF));
                            set_vw(&mut res, 1, 0x3F80_0000 | ((c >> 8) & 0xFF));
                            set_vw(&mut res, 2, 0x3F80_0000 | (c & 0xFF));
                            set_vw(&mut res, 3, 0x3F80_0000 | (c >> 24));
                        }
                        // VPACK_NORMSHORT2: signed shorts of word 3 -> (3.0 + hi*2^-22, 3.0 + lo*2^-22, 0.0, 1.0)
                        1 => {
                            let w = vw(&b, 3);
                            set_vw(&mut res, 0, unpack_short_as_3(((w >> 16) as u16) as i16));
                            set_vw(&mut res, 1, unpack_short_as_3((w as u16) as i16));
                            set_vf(&mut res, 2, 0.0);
                            set_vf(&mut res, 3, 1.0);
                        }
                        // VPACK_NORMSHORT4: words 2 and 3 -> four signed shorts as 3.0 + s*2^-22
                        4 => {
                            let (w2, w3) = (vw(&b, 2), vw(&b, 3));
                            set_vw(&mut res, 0, unpack_short_as_3(((w2 >> 16) as u16) as i16));
                            set_vw(&mut res, 1, unpack_short_as_3((w2 as u16) as i16));
                            set_vw(&mut res, 2, unpack_short_as_3(((w3 >> 16) as u16) as i16));
                            set_vw(&mut res, 3, unpack_short_as_3((w3 as u16) as i16));
                        }
                        // VPACK_FLOAT16_2: two halves in word 3 -> (x, y, 0.0, 1.0)
                        3 => {
                            let w = vw(&b, 3);
                            set_vf(&mut res, 0, half_to_f32((w >> 16) as u16));
                            set_vf(&mut res, 1, half_to_f32(w as u16));
                            set_vf(&mut res, 2, 0.0);
                            set_vf(&mut res, 3, 1.0);
                        }
                        // VPACK_FLOAT16_4: words 2 and 3 -> four halves
                        5 => {
                            let (w2, w3) = (vw(&b, 2), vw(&b, 3));
                            set_vf(&mut res, 0, half_to_f32((w2 >> 16) as u16));
                            set_vf(&mut res, 1, half_to_f32(w2 as u16));
                            set_vf(&mut res, 2, half_to_f32((w3 >> 16) as u16));
                            set_vf(&mut res, 3, half_to_f32(w3 as u16));
                        }
                        t => return Err(format!("Unimplemented VMX128 vupkd3d128 type {} (insn=0x{:08X}) at PC=0x{:08X}", t, insn, pc)),
                    },
                    _ => {
                        if (insn & 0x630) == 0x210 {
                            // vpermwi128: 8-bit permute = PERMh(bits 8..6) << 5 | PERMl(bits 20..16)
                            let perm = (((insn >> 6) & 0x7) << 5) | imm;
                            for i in 0..4 {
                                let sel = ((perm >> (6 - 2 * i)) & 3) as usize;
                                set_vw(&mut res, i, vw(&b, sel));
                            }
                        } else if (insn & 0x730) == 0x610 {
                            return Err(format!("Unimplemented VMX128 vpkd3d128 (insn=0x{:08X}) at PC=0x{:08X}", insn, pc));
                        } else if (insn & 0x730) == 0x710 {
                            // vrlimi128: rotate VB left by z words, then insert words selected by mask (imm[3:0], MSB = word 0)
                            let z = ((insn >> 6) & 3) as usize;
                            let mask = imm & 0xF;
                            res = self.regs.vr[vd];
                            for i in 0..4 {
                                if (mask >> (3 - i)) & 1 != 0 {
                                    set_vw(&mut res, i, vw(&b, (i + z) & 3));
                                }
                            }
                        } else if !bit4 {
                            match x4 {
                                // vcmp*128 (xo3 = bits 9..7, R = bit 6)
                                _ if ((insn >> 7) & 7) <= 4 => {
                                    let sub = (insn >> 7) & 7;
                                    let rc = ((insn >> 6) & 1) != 0;
                                    let mut all_true = true;
                                    let mut none_true = true;
                                    for i in 0..4 {
                                        let (fa, fb) = (vf(&a, i), vf(&b, i));
                                        let w: u32 = match sub {
                                            0 => if fa == fb { !0 } else { 0 },
                                            1 => if fa >= fb { !0 } else { 0 },
                                            2 => if fa > fb { !0 } else { 0 },
                                            3 => {
                                                // vcmpbfp128: bit0 = !(a <= b), bit1 = !(a >= -b)
                                                let le = fa <= fb;
                                                let ge = fa >= -fb;
                                                ((!le as u32) << 31) | ((!ge as u32) << 30)
                                            }
                                            _ => if vw(&a, i) == vw(&b, i) { !0 } else { 0 },
                                        };
                                        if sub == 3 {
                                            if w != 0 { none_true = false; }
                                            all_true = false;
                                        } else {
                                            if w == 0 { all_true = false; } else { none_true = false; }
                                        }
                                        set_vw(&mut res, i, w);
                                    }
                                    if rc {
                                        let bits = ((all_true as u32) << 3) | ((none_true as u32) << 1);
                                        self.regs.cr = (self.regs.cr & !(0xF << 4)) | (bits << 4);
                                    }
                                }
                                0b1010 => for i in 0..4 { set_vf(&mut res, i, fmax_ppc(vf(&a, i), vf(&b, i))); },
                                0b1011 => for i in 0..4 { set_vf(&mut res, i, fmin_ppc(vf(&a, i), vf(&b, i))); },
                                // vmrghw128 / vmrglw128
                                0b1100 | 0b1101 => {
                                    let o = if x4 == 0b1100 { 0 } else { 2 };
                                    set_vw(&mut res, 0, vw(&a, o));
                                    set_vw(&mut res, 1, vw(&b, o));
                                    set_vw(&mut res, 2, vw(&a, o + 1));
                                    set_vw(&mut res, 3, vw(&b, o + 1));
                                }
                                _ => return Err(format!("Unimplemented VMX128 opcode 6 (insn=0x{:08X}) at PC=0x{:08X}", insn, pc)),
                            }
                        } else {
                            match x4 {
                                // vsro128: shift right by octets (count in VB byte 15 bits 3..6)
                                0b1111 => {
                                    let sh = ((b[15] >> 3) & 0xF) as usize;
                                    for i in sh..16 { res[i] = a[i - sh]; }
                                }
                                // vrlw128 / vslw128 / vsraw128 / vsrw128: per-word shifts by VB low 5 bits
                                0b0001 | 0b0011 | 0b0101 | 0b0111 => {
                                    for i in 0..4 {
                                        let x = vw(&a, i);
                                        let s = vw(&b, i) & 31;
                                        let r = match x4 {
                                            0b0001 => x.rotate_left(s),
                                            0b0011 => x << s,
                                            0b0101 => ((x as i32) >> s) as u32,
                                            _ => x >> s,
                                        };
                                        set_vw(&mut res, i, r);
                                    }
                                }
                                _ => return Err(format!("Unimplemented VMX128 opcode 6 (insn=0x{:08X}) at PC=0x{:08X}", insn, pc)),
                            }
                        }
                    }
                }
                self.regs.vr[vd] = res;
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

                    // 983: stfiwx (Store Floating-Point as Integer Word Indexed)
                    983 => {
                        let frs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let val = (self.regs.fpr[frs].to_bits() & 0xFFFF_FFFF) as u32;
                        bus.write_u32(ea, val)?;
                    }

                    // 6: lvsl (Load Vector for Shift Left)
                    6 => {
                        let vd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let sh = (ea & 0xF) as u8;
                        for i in 0..16 {
                            self.regs.vr[vd][i] = (i as u8).wrapping_add(sh);
                        }
                    }

                    // 38: lvsr (Load Vector for Shift Right)
                    38 => {
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

                    // 39: lvehx (Load Vector Element Halfword Indexed). NOTE: xo 19 is mfcr/mfocrf, handled below.
                    39 => {
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

                    // 71: lvewx (Load Vector Element Word Indexed)
                    71 => {
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

                    // ---- Integer loads/stores: update forms, algebraic loads, byte-reversed ----
                    // 55: lwzux | 119: lbzux | 311: lhzux | 373: lwaux | 375: lhaux | 53: ldux
                    55 | 119 | 311 | 373 | 375 | 53 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]);
                        let val = match xo {
                            55 => bus.read_u32(ea)? as u64,
                            119 => bus.read_u8(ea)? as u64,
                            311 => bus.read_u16(ea)? as u64,
                            373 => bus.read_u32(ea)? as i32 as i64 as u64,
                            375 => bus.read_u16(ea)? as i16 as i64 as u64,
                            _ => bus.read_u64(ea)?,
                        };
                        self.regs.gpr[rd] = val;
                        self.regs.gpr[ra] = ea;
                    }
                    // 341: lwax | 343: lhax
                    341 | 343 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        self.regs.gpr[rd] = if xo == 341 { bus.read_u32(ea)? as i32 as i64 as u64 } else { bus.read_u16(ea)? as i16 as i64 as u64 };
                    }
                    // 183: stwux | 247: stbux | 439: sthux | 181: stdux
                    183 | 247 | 439 | 181 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]);
                        let v = self.regs.gpr[rs];
                        match xo {
                            183 => bus.write_u32(ea, v as u32)?,
                            247 => bus.write_u8(ea, v as u8)?,
                            439 => bus.write_u16(ea, v as u16)?,
                            _ => bus.write_u64(ea, v)?,
                        }
                        self.regs.gpr[ra] = ea;
                    }
                    // 534: lwbrx | 790: lhbrx | 532: ldbrx
                    534 | 790 | 532 => {
                        let rd = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        self.regs.gpr[rd] = match xo {
                            534 => bus.read_u32(ea)?.swap_bytes() as u64,
                            790 => bus.read_u16(ea)?.swap_bytes() as u64,
                            _ => bus.read_u64(ea)?.swap_bytes(),
                        };
                    }
                    // 662: stwbrx | 918: sthbrx | 660: stdbrx
                    662 | 918 | 660 => {
                        let rs = ((insn >> 21) & 0x1F) as usize;
                        let ra = ((insn >> 16) & 0x1F) as usize;
                        let rb = ((insn >> 11) & 0x1F) as usize;
                        let ea = if ra == 0 { self.regs.gpr[rb] } else { self.regs.gpr[ra].wrapping_add(self.regs.gpr[rb]) };
                        let v = self.regs.gpr[rs];
                        match xo {
                            662 => bus.write_u32(ea, (v as u32).swap_bytes())?,
                            918 => bus.write_u16(ea, (v as u16).swap_bytes())?,
                            _ => bus.write_u64(ea, v.swap_bytes())?,
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

                    _ => return Err(format!("Unimplemented opcode 31 xo {} (insn=0x{:08X})", xo, insn)),
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


// ---- VMX128 helpers ----
#[inline(always)]
fn vw(v: &[u8; 16], i: usize) -> u32 {
    u32::from_be_bytes([v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]])
}
#[inline(always)]
fn set_vw(v: &mut [u8; 16], i: usize, w: u32) {
    v[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
}
#[inline(always)]
fn vf(v: &[u8; 16], i: usize) -> f32 {
    f32::from_bits(vw(v, i))
}
#[inline(always)]
fn set_vf(v: &mut [u8; 16], i: usize, f: f32) {
    set_vw(v, i, f.to_bits());
}
fn round_ties_even_f32(x: f32) -> f32 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && (r % 2.0) != 0.0 { r - x.signum() } else { r }
}
/// AltiVec vmaxfp: if either operand is NaN the result is that NaN, otherwise the larger value.
fn fmax_ppc(a: f32, b: f32) -> f32 {
    if a.is_nan() { a } else if b.is_nan() { b } else if a > b { a } else { b }
}
fn fmin_ppc(a: f32, b: f32) -> f32 {
    if a.is_nan() { a } else if b.is_nan() { b } else if a < b { a } else { b }
}
/// IEEE 754 binary16 -> binary32
fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1F) as u32;
    let man = (h & 0x3FF) as u32;
    let bits = if exp == 0 {
        if man == 0 {
            sign << 31
        } else {
            // subnormal: normalize
            let mut e = 127 - 15 + 1;
            let mut m = man;
            while m & 0x400 == 0 { m <<= 1; e -= 1; }
            (sign << 31) | ((e as u32) << 23) | ((m & 0x3FF) << 13)
        }
    } else if exp == 0x1F {
        (sign << 31) | 0x7F80_0000 | (man << 13)
    } else {
        (sign << 31) | ((exp + 127 - 15) << 23) | (man << 13)
    };
    f32::from_bits(bits)
}
/// vupkd3d NORMSHORT helper: bits of 3.0f plus the sign-extended short; -32768 yields a quiet NaN (hardware overflow marker).
fn unpack_short_as_3(s: i16) -> u32 {
    let r = 0x4040_0000u32.wrapping_add(s as i32 as u32);
    if r == 0x403F_8000 { 0x7FC0_0000 } else { r }
}