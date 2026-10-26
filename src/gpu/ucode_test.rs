//! Test-only microcode encoders (the inverse of the layouts in gpu/ucode.rs), shared by the disassembler and
//! interpreter tests. Everything encoded here is synthetic.

use super::ucode::{SCALAR_RETAIN_PREV, VECTOR_MAX};

/// A control flow instruction as (dword_0, 16-bit dword_1).
pub type Cf = (u32, u32);

pub fn cf(opcode: u32, dw0: u32, dw1_low12: u32) -> Cf {
    (dw0, (opcode << 12) | dw1_low12)
}

pub fn exec(opcode: u32, address: u32, count: u32, sequence: u32, dw1_low12: u32) -> Cf {
    cf(opcode, address | (count << 12) | (sequence << 16), dw1_low12)
}

pub fn alloc(kind: u32, size: u32) -> Cf {
    cf(12, size, kind << 9)
}

/// The inverse of unpack_cf_pair.
pub fn pack(a: Cf, b: Cf) -> [u32; 3] {
    [a.0, (a.1 & 0xFFFF) | (b.0 << 16), (b.0 >> 16) | (b.1 << 16)]
}

#[derive(Default, Clone, Copy)]
pub struct Alu {
    pub vdst: u32,
    pub vdst_rel: bool,
    pub sdst: u32,
    pub sdst_rel: bool,
    pub export: bool,
    pub vmask: u32,
    pub smask: u32,
    pub vclamp: bool,
    pub sclamp: bool,
    pub sop: u32,
    pub vop: u32,
    pub swz: [u32; 3],
    pub neg: [bool; 3],
    pub reg: [u32; 3],
    pub temp: [bool; 3],
    pub abs_constants: bool,
    pub const_0_rel: bool,
    pub const_1_rel: bool,
    pub a0_relative: bool,
    pub predicated: bool,
    pub pred_condition: bool,
}

pub fn alu(a: Alu) -> [u32; 3] {
    let b = |v: bool| v as u32;
    [
        a.vdst | b(a.vdst_rel) << 6 | b(a.abs_constants) << 7 | a.sdst << 8 | b(a.sdst_rel) << 14 | b(a.export) << 15
            | a.vmask << 16 | a.smask << 20 | b(a.vclamp) << 24 | b(a.sclamp) << 25 | a.sop << 26,
        a.swz[2] | a.swz[1] << 8 | a.swz[0] << 16 | b(a.neg[2]) << 24 | b(a.neg[1]) << 25 | b(a.neg[0]) << 26
            | b(a.pred_condition) << 27 | b(a.predicated) << 28 | b(a.a0_relative) << 29 | b(a.const_1_rel) << 30
            | b(a.const_0_rel) << 31,
        a.reg[2] | a.reg[1] << 8 | a.reg[0] << 16 | a.vop << 24 | b(a.temp[2]) << 29 | b(a.temp[1]) << 30 | b(a.temp[0]) << 31,
    ]
}

/// Vector max r0._, r0, r0 with retain_prev: the default nop.
pub fn alu_nop() -> [u32; 3] {
    alu(Alu { vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, temp: [true; 3], ..Default::default() })
}

pub fn shader(parts: &[[u32; 3]]) -> Vec<u32> {
    parts.iter().flatten().copied().collect()
}
