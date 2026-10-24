//! AltiVec/VMX128 (primary opcodes 4/5/6) and the indexed vector loads/stores of opcode 31.

use crate::cpu::interpreter::{Cpu, StepResult};
use crate::memory::MemoryBus;

impl Cpu {
    /// Primary opcodes: 4 | 5 | 6.
    #[inline(always)]
    pub(in crate::cpu) fn exec_vmx(&mut self, bus: &mut MemoryBus, insn: u32, pc: u64, opcode: u32) -> Result<StepResult, String> {
        match opcode {

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

            _ => unreachable!("exec_vmx: opcode {} not in this group", opcode),
        }

        Ok(StepResult::Ok)
    }

    /// Opcode 31, xo: 6 | 38 | 103 | 359 | 231 | 487 | 3 | 7 | 39 | 71 | 67 | 135 | 167 | 70 | 134 | 199 | 519 | 775 | 551 | 807 | 647 | 903 | 679 | 935.
    #[inline(always)]
    pub(in crate::cpu) fn exec_vmx_x31(&mut self, bus: &mut MemoryBus, insn: u32, _pc: u64, xo: u32) -> Result<StepResult, String> {
        match xo {

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

            _ => unreachable!("exec_vmx_x31: xo {} not in this group", xo),
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
