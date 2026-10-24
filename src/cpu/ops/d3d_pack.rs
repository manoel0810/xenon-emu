// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/cpu/ppc/ppc_emit_altivec.cc   InstrEmit_vpkd3d128 / InstrEmit_vupkd3d128 (types, pack/shift masks)
//   src/xenia/cpu/backend/x64/x64_seq_vector.cc  PACK / UNPACK / PERMUTE_I32 sequences (bit-level semantics)
//   src/xenia/cpu/backend/x64/x64_emitter.cc     XMMPack* / XMMUnpack* constants
//   src/xenia/cpu/hir/opcodes.h              MakePermuteMask
//   src/xenia/base/math.h                    float_to_xenos_half / xenos_half_to_float
//
//! VMX128 D3D pack/unpack (vpkd3d128 / vupkd3d128). Vectors are four 32-bit lanes [x, y, z, w] (word 0..3 of
//! the register). The packers expect the "biased" float encoding the D3D runtime produces, 3.0 + n * 2^-22
//! (0x40400000 + n), saturate in float, then take the low bits; the unpackers produce the same encoding.

/// Quiet NaN returned by the unpackers on negative overflow (XMMQNaN).
const QNAN: u32 = 0x7FC0_0000;
const F3: u32 = 0x4040_0000; // 3.0f
const F1: u32 = 0x3F80_0000; // 1.0f

/// vmaxps(src, lo) then vminps(.., hi): NaN saturates to `lo` (max returns the second operand on NaN).
fn sat(v: u32, lo: u32, hi: u32) -> u32 {
    let (v, lo, hi) = (f32::from_bits(v), f32::from_bits(lo), f32::from_bits(hi));
    let t = if v > lo { v } else { lo };
    (if t < hi { t } else { hi }).to_bits()
}

/// float_to_xenos_half: no inf/NaN (saturates to 0x7FFF), denormals flushed unless `preserve_denormal`.
pub fn float_to_xenos_half(value: f32, preserve_denormal: bool, round_to_nearest_even: bool) -> u16 {
    let integer_value = value.to_bits();
    let abs_value = integer_value & 0x7FFF_FFFF;
    let mut result: u32;
    if abs_value >= 0x47FF_E000 {
        result = 0x7FFF;
    } else {
        if abs_value < 0x3880_0000 {
            if preserve_denormal {
                let shift = (113u32.wrapping_sub(abs_value >> 23)).min(24);
                result = (0x80_0000 | (abs_value & 0x7F_FFFF)) >> shift;
            } else {
                result = 0;
            }
        } else {
            result = abs_value.wrapping_add(0xC800_0000);
        }
        if round_to_nearest_even {
            result = result.wrapping_add(0xFFF + ((result >> 13) & 1));
        }
        result = (result >> 13) & 0x7FFF;
    }
    (result | ((integer_value & 0x8000_0000) >> 16)) as u16
}

/// xenos_half_to_float: exponent 31 is an ordinary exponent (no inf/NaN); denormals flush to zero unless
/// `preserve_denormal`.
pub fn xenos_half_to_float(value: u16, preserve_denormal: bool) -> f32 {
    let mut mantissa = (value & 0x3FF) as u32;
    let mut exponent = ((value >> 10) & 0x1F) as u32;
    if exponent == 0 {
        if !preserve_denormal {
            mantissa = 0;
        } else if mantissa != 0 {
            let lz = mantissa.leading_zeros() - (32 - 11);
            exponent = (1 - lz as i32) as u32;
            mantissa = (mantissa << lz) & 0x3FF;
        }
        if mantissa == 0 {
            exponent = (-112i32) as u32;
        }
    }
    let result = (((value & 0x8000) as u32) << 16) | (exponent.wrapping_add(112) << 23) | (mantissa << 13);
    f32::from_bits(result)
}

/// OPCODE_PACK for the D3D types (vpkd3d128 `type` = IMM >> 2). `None` = unhandled type.
pub fn pack(ty: u32, s: [u32; 4]) -> Option<[u32; 4]> {
    Some(match ty {
        // VPACK_D3DCOLOR: saturate to [3.0, 0x404000FF]; w = A<<24 | R<<16 | G<<8 | B (RGBA = xyzw)
        0 => {
            let c: Vec<u32> = s.iter().map(|&v| sat(v, F3, 0x4040_00FF) & 0xFF).collect();
            [0, 0, 0, (c[3] << 24) | (c[0] << 16) | (c[1] << 8) | c[2]]
        }
        // VPACK_NORMSHORT2: saturate to [0x403F8001, 0x40407FFF]; w = x16 << 16 | y16
        1 => {
            let c: Vec<u32> = s.iter().map(|&v| sat(v, 0x403F_8001, 0x4040_7FFF) & 0xFFFF).collect();
            [0, 0, 0, (c[0] << 16) | c[1]]
        }
        // VPACK_NORMPACKED32 2_10_10_10 w_z_y_x: every lane = x | y<<10 | z<<20 | w<<30
        2 => {
            let lo = [0x403F_FE01, 0x403F_FE01, 0x403F_FE01, 0x4040_0000];
            let hi = [0x4040_01FF, 0x4040_01FF, 0x4040_01FF, 0x4040_0003];
            let mask = [0x3FF, 0x3FF, 0x3FF, 0x3];
            let m: Vec<u32> = (0..4).map(|i| sat(s[i], lo[i], hi[i]) & mask[i]).collect();
            let v = m[0] | (m[1] << 10) | (m[2] << 20) | (m[3] << 30);
            [v, v, v, v]
        }
        // VPACK_FLOAT16_2: w = half(x) << 16 | half(y) (truncating conversion)
        3 => {
            let h = |i: usize| float_to_xenos_half(f32::from_bits(s[i]), false, false) as u32;
            [0, 0, 0, (h(0) << 16) | h(1)]
        }
        // VPACK_NORMSHORT4: z = x16 << 16 | y16, w = z16 << 16 | w16
        4 => {
            let c: Vec<u32> = s.iter().map(|&v| sat(v, 0x403F_8001, 0x4040_7FFF) & 0xFFFF).collect();
            [0, 0, (c[0] << 16) | c[1], (c[2] << 16) | c[3]]
        }
        // VPACK_FLOAT16_4: z = half(x) << 16 | half(y), w = half(z) << 16 | half(w) (round to nearest even)
        5 => {
            let h = |i: usize| float_to_xenos_half(f32::from_bits(s[i]), false, true) as u32;
            [0, 0, (h(0) << 16) | h(1), (h(2) << 16) | h(3)]
        }
        // VPACK_NORMPACKED64 4_20_20_20 w_z_y_x: z:w (64 bits) = w:4 | z:20 | y:20 | x:20
        6 => {
            let lo = [0x4038_0001, 0x4038_0001, 0x4038_0001, 0x4040_0000];
            let hi = [0x4047_FFFF, 0x4047_FFFF, 0x4047_FFFF, 0x4040_000F];
            let mask = [0xF_FFFF, 0xF_FFFF, 0xF_FFFF, 0xF];
            let m: Vec<u32> = (0..4).map(|i| sat(s[i], lo[i], hi[i]) & mask[i]).collect();
            [0, 0, (m[3] << 28) | (m[2] << 8) | (m[1] >> 12), m[0] | ((m[1] & 0xFFF) << 20)]
        }
        _ => return None,
    })
}

/// Sign-extends the low `bits` bits, adds `base` as an integer and maps `overflow` to a quiet NaN.
fn unbias(v: u32, bits: u32, base: u32, overflow: u32) -> u32 {
    let sh = 32 - bits;
    let r = ((((v << sh) as i32) >> sh) as u32).wrapping_add(base);
    if r == overflow { QNAN } else { r }
}

/// OPCODE_UNPACK for the D3D types (vupkd3d128 `type` = IMM >> 2). `None` = unhandled type.
pub fn unpack(ty: u32, s: [u32; 4]) -> Option<[u32; 4]> {
    Some(match ty {
        // VPACK_D3DCOLOR: w = 0xAARRGGBB -> (R, G, B, A) as 1.0f | byte
        0 => {
            let c = s[3];
            [F1 | ((c >> 16) & 0xFF), F1 | ((c >> 8) & 0xFF), F1 | (c & 0xFF), F1 | (c >> 24)]
        }
        // VPACK_NORMSHORT2: (3.0 + hi(w)*2^-22, 3.0 + lo(w)*2^-22, 0.0, 1.0); -32768 -> QNaN
        1 => [unbias(s[3] >> 16, 16, F3, 0x403F_8000), unbias(s[3] & 0xFFFF, 16, F3, 0x403F_8000), 0, F1],
        // VPACK_NORMPACKED32: w = A:2 B:10 G:10 R:10 -> (3+R, 3+G, 3+B signed 10-bit; 1.0 + A); -512 -> QNaN
        2 => {
            let w = s[3];
            [
                unbias(w & 0x3FF, 10, F3, 0x403F_FE00),
                unbias((w >> 10) & 0x3FF, 10, F3, 0x403F_FE00),
                unbias((w >> 20) & 0x3FF, 10, F3, 0x403F_FE00),
                (w >> 30).wrapping_add(F1),
            ]
        }
        // VPACK_FLOAT16_2: (half(hi w), half(lo w), 0.0, 1.0)
        3 => {
            let h = |v: u32| xenos_half_to_float(v as u16, false).to_bits();
            [h(s[3] >> 16), h(s[3] & 0xFFFF), 0, F1]
        }
        // VPACK_NORMSHORT4: z, w -> four signed shorts as 3.0 + s*2^-22
        4 => {
            let u = |v: u32| unbias(v, 16, F3, 0x403F_8000);
            [u(s[2] >> 16), u(s[2] & 0xFFFF), u(s[3] >> 16), u(s[3] & 0xFFFF)]
        }
        // VPACK_FLOAT16_4: z, w -> four halves
        5 => {
            let h = |v: u32| xenos_half_to_float(v as u16, false).to_bits();
            [h(s[2] >> 16), h(s[2] & 0xFFFF), h(s[3] >> 16), h(s[3] & 0xFFFF)]
        }
        // VPACK_NORMPACKED64: z:w = w:4 z:20 y:20 x:20 -> (3+x, 3+y, 3+z signed 20-bit; 1.0 + w); -2^19 -> QNaN
        6 => {
            let v = ((s[2] as u64) << 32) | s[3] as u64;
            let f = |sh: u32| unbias(((v >> sh) & 0xF_FFFF) as u32, 20, F3, 0x4038_0000);
            [f(0), f(20), f(40), ((v >> 60) as u32).wrapping_add(F1)]
        }
        _ => return None,
    })
}

/// MakePermuteMask(sel_x, x, sel_y, y, sel_z, z, sel_w, w) as (selector, lane) pairs; selector 0 = old VD,
/// 1 = packed value (PERMUTE_I32 between src2 = VD and src3 = packed).
type Mask = [(u32, usize); 4];

/// Lane selection of vpkd3d128 for `pack` = IMM & 3 and `shift` = z. `None` = unhandled (pack 0).
fn pack_mask(pack: u32, shift: u32) -> Option<Mask> {
    Some(match (pack, shift) {
        // VPACK_32: shift = 3 puts the packed 32 bits in x
        (1, 0) => [(0, 0), (0, 1), (0, 2), (1, 3)],
        (1, 1) => [(0, 0), (0, 1), (1, 3), (0, 3)],
        (1, 2) => [(0, 0), (1, 3), (0, 2), (0, 3)],
        (1, 3) => [(1, 3), (0, 1), (0, 2), (0, 3)],
        // 64-bit
        (2, 0) | (3, 0) => [(0, 0), (0, 1), (1, 2), (1, 3)],
        (2, 1) | (3, 1) => [(0, 0), (1, 2), (1, 3), (0, 3)],
        (2, 2) | (3, 2) => [(1, 2), (1, 3), (0, 2), (0, 3)],
        (2, 3) => [(1, 3), (0, 1), (0, 2), (0, 3)],
        (3, 3) => [(0, 0), (0, 1), (0, 2), (1, 2)],
        _ => return None,
    })
}

/// vpkd3d128 VD, VB with IMM (type << 2 | pack) and z (shift): pack VB and merge it into VD.
/// Err(()) for the combinations Xenia leaves unhandled (unknown type, pack 0).
pub fn vpkd3d128(vd: [u32; 4], vb: [u32; 4], imm: u32, shift: u32) -> Result<[u32; 4], ()> {
    let packed = pack(imm >> 2, vb).ok_or(())?;
    let mask = pack_mask(imm & 3, shift).ok_or(())?;
    let mut out = [0u32; 4];
    for (k, &(sel, lane)) in mask.iter().enumerate() {
        out[k] = if sel == 0 { vd[lane] } else { packed[lane] };
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Biased float 3.0 + n * 2^-22, the encoding the D3D runtime feeds to the packers.
    fn b(n: i32) -> u32 {
        (F3 as i32 + n) as u32
    }
    fn f(x: f32) -> u32 {
        x.to_bits()
    }

    #[test]
    fn half_conversions() {
        assert_eq!(float_to_xenos_half(1.0, false, false), 0x3C00);
        assert_eq!(float_to_xenos_half(-2.0, false, false), 0xC000);
        // no infinities: saturates to the largest magnitude
        assert_eq!(float_to_xenos_half(1.0e9, false, false), 0x7FFF);
        assert_eq!(float_to_xenos_half(f32::INFINITY, false, false), 0x7FFF);
        // denormals flush to zero by default
        assert_eq!(float_to_xenos_half(1.0e-6, false, false), 0);
        // rounding: 1 + 2^-11 truncates to 1.0 and rounds (ties to even) to 1.0; 1 + 3*2^-11 rounds up
        assert_eq!(float_to_xenos_half(1.0 + 1.0 / 2048.0, false, true), 0x3C00);
        assert_eq!(float_to_xenos_half(1.0 + 3.0 / 2048.0, false, false), 0x3C01);
        assert_eq!(float_to_xenos_half(1.0 + 3.0 / 2048.0, false, true), 0x3C02);
        assert_eq!(xenos_half_to_float(0x3C00, false), 1.0);
        // exponent 31 is a normal number (2^16 * 1.0), not infinity
        assert_eq!(xenos_half_to_float(0x7C00, false), 65536.0);
        assert_eq!(xenos_half_to_float(0x0001, false), 0.0);
        assert_eq!(xenos_half_to_float(0x8000, false).to_bits(), 0x8000_0000);
    }

    #[test]
    fn d3dcolor_pack_and_unpack() {
        // RGBA = (0x11, 0x22, 0x33, 0x44) -> w = 0x44112233
        let p = pack(0, [b(0x11), b(0x22), b(0x33), b(0x44)]).unwrap();
        assert_eq!(p, [0, 0, 0, 0x4411_2233]);
        // saturation: below 3.0 -> 0, above 0x404000FF -> 0xFF, NaN -> 0 (max returns 3.0)
        assert_eq!(pack(0, [f(0.5), b(0x1000), QNAN, b(0xFF)]).unwrap()[3], 0xFF00_FF00);
        assert_eq!(unpack(0, p).unwrap(), [F1 | 0x11, F1 | 0x22, F1 | 0x33, F1 | 0x44]);
    }

    #[test]
    fn short_pack_and_unpack() {
        assert_eq!(pack(1, [b(-2), b(0x1234), 0, 0]).unwrap(), [0, 0, 0, 0xFFFE_1234]);
        // saturates to +-32767
        assert_eq!(pack(1, [b(40000), b(-40000), 0, 0]).unwrap()[3], 0x7FFF_8001);
        assert_eq!(pack(4, [b(1), b(-1), b(2), b(-2)]).unwrap(), [0, 0, 0x0001_FFFF, 0x0002_FFFE]);
        assert_eq!(unpack(1, [0, 0, 0, 0xFFFE_1234]).unwrap(), [b(-2), b(0x1234), 0, F1]);
        // -32768 is the overflow marker -> QNaN
        assert_eq!(unpack(4, [0, 0, 0x8000_0001, 0x7FFF_0000]).unwrap(), [QNAN, b(1), b(0x7FFF), b(0)]);
    }

    #[test]
    fn packed32_2101010() {
        let p = pack(2, [b(5), b(-1), b(511), b(2)]).unwrap();
        let v = 5 | (0x3FF << 10) | (511 << 20) | (2 << 30);
        assert_eq!(p, [v, v, v, v]);
        // saturation: xyz to [-511, 511], w to [0, 3]
        let s = pack(2, [b(1000), b(-1000), b(0), b(9)]).unwrap()[0];
        assert_eq!((s & 0x3FF, (s >> 10) & 0x3FF, s >> 30), (511, 0x201, 3));
        assert_eq!(unpack(2, [0, 0, 0, v]).unwrap(), [b(5), b(-1), b(511), F1 + 2]);
        // -512 -> QNaN
        assert_eq!(unpack(2, [0, 0, 0, 0x200]).unwrap()[0], QNAN);
    }

    #[test]
    fn packed64_4202020() {
        let p = pack(6, [b(0x12345), b(-3), b(0x7FFFF), b(9)]).unwrap();
        let v: u64 = 0x12345 | ((((-3i64) as u64) & 0xFFFFF) << 20) | (0x7FFFFu64 << 40) | (9u64 << 60);
        assert_eq!((p[2], p[3]), ((v >> 32) as u32, v as u32));
        assert_eq!(unpack(6, [0, 0, (v >> 32) as u32, v as u32]).unwrap(), [b(0x12345), b(-3), b(0x7FFFF), F1 + 9]);
        // -2^19 -> QNaN
        assert_eq!(unpack(6, [0, 0, 0, 0x8_0000]).unwrap()[0], QNAN);
    }

    #[test]
    fn float16_pack_and_unpack() {
        assert_eq!(pack(3, [f(1.0), f(-2.0), 0, 0]).unwrap(), [0, 0, 0, 0x3C00_C000]);
        assert_eq!(pack(5, [f(1.0), f(0.5), f(-1.0), f(65536.0)]).unwrap(), [0, 0, 0x3C00_3800, 0xBC00_7C00]);
        assert_eq!(unpack(3, [0, 0, 0, 0x3C00_C000]).unwrap(), [f(1.0), f(-2.0), 0, F1]);
        assert_eq!(unpack(5, [0, 0, 0x3C00_3800, 0xBC00_7C00]).unwrap(), [f(1.0), f(0.5), f(-1.0), f(65536.0)]);
    }

    #[test]
    fn vpkd3d128_merges_into_vd() {
        let vd = [0xA0, 0xA1, 0xA2, 0xA3];
        let rgba = [b(0x11), b(0x22), b(0x33), b(0x44)];
        // the dash's instruction 0x1BE166D4: D3DCOLOR, VPACK_32, shift 3 -> packed color in x
        assert_eq!(vpkd3d128(vd, rgba, 1, 3), Ok([0x4411_2233, 0xA1, 0xA2, 0xA3]));
        assert_eq!(vpkd3d128(vd, rgba, 1, 0), Ok([0xA0, 0xA1, 0xA2, 0x4411_2233]));
        assert_eq!(vpkd3d128(vd, rgba, 1, 1), Ok([0xA0, 0xA1, 0x4411_2233, 0xA3]));
        assert_eq!(vpkd3d128(vd, rgba, 1, 2), Ok([0xA0, 0x4411_2233, 0xA2, 0xA3]));
        // 64-bit (NORMSHORT4 = type 4, pack 2): packed z:w placed by shift
        let s4 = [b(1), b(2), b(3), b(4)];
        let (pz, pw) = (0x0001_0002, 0x0003_0004);
        assert_eq!(vpkd3d128(vd, s4, (4 << 2) | 2, 0), Ok([0xA0, 0xA1, pz, pw]));
        assert_eq!(vpkd3d128(vd, s4, (4 << 2) | 2, 1), Ok([0xA0, pz, pw, 0xA3]));
        assert_eq!(vpkd3d128(vd, s4, (4 << 2) | 2, 2), Ok([pz, pw, 0xA2, 0xA3]));
        assert_eq!(vpkd3d128(vd, s4, (4 << 2) | 2, 3), Ok([pw, 0xA1, 0xA2, 0xA3]));
        // pack 3 differs from pack 2 only at shift 3
        assert_eq!(vpkd3d128(vd, s4, (4 << 2) | 3, 3), Ok([0xA0, 0xA1, 0xA2, pz]));
        // unhandled: pack 0, type 7
        assert_eq!(vpkd3d128(vd, s4, 4 << 2, 0), Err(()));
        assert_eq!(vpkd3d128(vd, s4, (7 << 2) | 1, 0), Err(()));
    }
}
