//! Integer and floating-point loads/stores (D, DS and X forms, update, byte-reversed, reserve/conditional).

use crate::cpu::interpreter::{Cpu, StepResult};
use crate::memory::MemoryBus;

impl Cpu {
    /// Primary opcodes: 32 | 33 | 34 | 35 | 36 | 37 | 38 | 39 | 40 | 41 | 42 | 43 | 44 | 45 | 48 | 49 | 50 | 51 | 52 | 53 | 54 | 55 | 58 | 62.
    #[inline(always)]
    pub(in crate::cpu) fn exec_loadstore(&mut self, bus: &mut MemoryBus, insn: u32, _pc: u64, opcode: u32) -> Result<StepResult, String> {
        match opcode {

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

            _ => unreachable!("exec_loadstore: opcode {} not in this group", opcode),
        }

        Ok(StepResult::Ok)
    }

    /// Opcode 31, xo: 20 | 84 | 150 | 214 | 23 | 87 | 279 | 21 | 151 | 215 | 407 | 149 | 535 | 599 | 663 | 727 | 983 | 55 | 119 | 311 | 373 | 375 | 53 | 341 | 343 | 183 | 247 | 439 | 181 | 534 | 790 | 532 | 662 | 918 | 660.
    #[inline(always)]
    pub(in crate::cpu) fn exec_loadstore_x31(&mut self, bus: &mut MemoryBus, insn: u32, _pc: u64, xo: u32) -> Result<StepResult, String> {
        match xo {

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

            _ => unreachable!("exec_loadstore_x31: xo {} not in this group", xo),
        }

        Ok(StepResult::Ok)
    }
}
