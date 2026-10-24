//! AltiVec (VMX) VX-form instructions of primary opcode 4 (`xo = insn & 0x7FF`).
//! VA-form (xo 32..47) and VMX128 forms are handled by the interpreter itself.
//! Vector registers hold big-endian lane data: byte 0 is the most significant byte of word 0.

use crate::cpu::registers::Registers;

#[allow(dead_code)]
#[inline] fn b_of(v: &[u8; 16], i: usize) -> u8 { v[i] }
#[inline] fn h_of(v: &[u8; 16], i: usize) -> u16 { u16::from_be_bytes([v[i * 2], v[i * 2 + 1]]) }
#[inline] fn w_of(v: &[u8; 16], i: usize) -> u32 { u32::from_be_bytes([v[i * 4], v[i * 4 + 1], v[i * 4 + 2], v[i * 4 + 3]]) }
#[inline] fn set_h(v: &mut [u8; 16], i: usize, x: u16) { v[i * 2..i * 2 + 2].copy_from_slice(&x.to_be_bytes()); }
#[inline] fn set_w(v: &mut [u8; 16], i: usize, x: u32) { v[i * 4..i * 4 + 4].copy_from_slice(&x.to_be_bytes()); }
#[inline] fn f_of(v: &[u8; 16], i: usize) -> f32 { f32::from_bits(w_of(v, i)) }
#[inline] fn set_f(v: &mut [u8; 16], i: usize, x: f32) { set_w(v, i, x.to_bits()); }

fn round_even(x: f32) -> f32 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && (r % 2.0) != 0.0 { r - x.signum() } else { r }
}
fn fmax(a: f32, b: f32) -> f32 { if a.is_nan() { a } else if b.is_nan() { b } else if a > b { a } else { b } }
fn fmin(a: f32, b: f32) -> f32 { if a.is_nan() { a } else if b.is_nan() { b } else if a < b { a } else { b } }

/// Executes one AltiVec VX-form instruction. Returns `Err` for opcodes that are not implemented.
pub fn exec_vx(regs: &mut Registers, insn: u32) -> Result<(), String> {
    let xo = insn & 0x7FF;
    let vd = ((insn >> 21) & 0x1F) as usize;
    let ua = (insn >> 16) & 0x1F; // UIMM/SIMM or VA index
    let va = ua as usize;
    let vb = ((insn >> 11) & 0x1F) as usize;
    let a = regs.vr[va];
    let b = regs.vr[vb];
    let mut r = [0u8; 16];
    let mut sat = false;

    // Compare group: base xo plus optional Rc bit (0x400)
    let cmp_base = xo & !0x400;
    if matches!(cmp_base, 6 | 70 | 134 | 198 | 454 | 518 | 582 | 646 | 710 | 774 | 838 | 902 | 966) {
        let rc = (xo & 0x400) != 0;
        let mut all = true;
        let mut none = true;
        let lanes = match cmp_base { 6 | 518 | 774 => 16, 70 | 582 | 838 => 8, _ => 4 };
        for i in 0..lanes {
            let t: bool = match cmp_base {
                6 => a[i] == b[i],
                518 => a[i] > b[i],
                774 => (a[i] as i8) > (b[i] as i8),
                70 => h_of(&a, i) == h_of(&b, i),
                582 => h_of(&a, i) > h_of(&b, i),
                838 => (h_of(&a, i) as i16) > (h_of(&b, i) as i16),
                134 => w_of(&a, i) == w_of(&b, i),
                646 => w_of(&a, i) > w_of(&b, i),
                902 => (w_of(&a, i) as i32) > (w_of(&b, i) as i32),
                198 => f_of(&a, i) == f_of(&b, i),
                454 => f_of(&a, i) >= f_of(&b, i),
                710 => f_of(&a, i) > f_of(&b, i),
                _ => {
                    // vcmpbfp: bit0 = !(a <= b), bit1 = !(a >= -b)
                    let (x, y) = (f_of(&a, i), f_of(&b, i));
                    let w = ((!(x <= y) as u32) << 31) | ((!(x >= -y) as u32) << 30);
                    set_w(&mut r, i, w);
                    if w != 0 { none = false; }
                    all = false;
                    continue;
                }
            };
            if t { none = false; } else { all = false; }
            match cmp_base {
                6 | 518 | 774 => r[i] = if t { 0xFF } else { 0 },
                70 | 582 | 838 => set_h(&mut r, i, if t { 0xFFFF } else { 0 }),
                _ => set_w(&mut r, i, if t { 0xFFFF_FFFF } else { 0 }),
            }
        }
        if rc {
            let bits = ((all as u32) << 3) | ((none as u32) << 1);
            regs.cr = (regs.cr & !(0xF << 4)) | (bits << 4);
        }
        regs.vr[vd] = r;
        return Ok(());
    }

    match xo {
        // ---- add / subtract ----
        0 => for i in 0..16 { r[i] = a[i].wrapping_add(b[i]); },
        64 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i).wrapping_add(h_of(&b, i))); },
        128 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i).wrapping_add(w_of(&b, i))); },
        10 => for i in 0..4 { set_f(&mut r, i, f_of(&a, i) + f_of(&b, i)); },
        384 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i).checked_add(w_of(&b, i)).is_none() as u32); },
        512 => for i in 0..16 { let s = a[i] as u32 + b[i] as u32; if s > 255 { sat = true; } r[i] = s.min(255) as u8; },
        576 => for i in 0..8 { let s = h_of(&a, i) as u32 + h_of(&b, i) as u32; if s > 0xFFFF { sat = true; } set_h(&mut r, i, s.min(0xFFFF) as u16); },
        640 => for i in 0..4 { let s = w_of(&a, i) as u64 + w_of(&b, i) as u64; if s > 0xFFFF_FFFF { sat = true; } set_w(&mut r, i, s.min(0xFFFF_FFFF) as u32); },
        768 => for i in 0..16 { let s = a[i] as i8 as i32 + b[i] as i8 as i32; if !(-128..=127).contains(&s) { sat = true; } r[i] = s.clamp(-128, 127) as i8 as u8; },
        832 => for i in 0..8 { let s = h_of(&a, i) as i16 as i32 + h_of(&b, i) as i16 as i32; if !(-32768..=32767).contains(&s) { sat = true; } set_h(&mut r, i, s.clamp(-32768, 32767) as i16 as u16); },
        896 => for i in 0..4 { let s = w_of(&a, i) as i32 as i64 + w_of(&b, i) as i32 as i64; if s < i32::MIN as i64 || s > i32::MAX as i64 { sat = true; } set_w(&mut r, i, s.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32); },
        1024 => for i in 0..16 { r[i] = a[i].wrapping_sub(b[i]); },
        1088 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i).wrapping_sub(h_of(&b, i))); },
        1152 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i).wrapping_sub(w_of(&b, i))); },
        74 => for i in 0..4 { set_f(&mut r, i, f_of(&a, i) - f_of(&b, i)); },
        1408 => for i in 0..4 { set_w(&mut r, i, (w_of(&a, i) >= w_of(&b, i)) as u32); },
        1536 => for i in 0..16 { let s = a[i] as i32 - b[i] as i32; if s < 0 { sat = true; } r[i] = s.max(0) as u8; },
        1600 => for i in 0..8 { let s = h_of(&a, i) as i32 - h_of(&b, i) as i32; if s < 0 { sat = true; } set_h(&mut r, i, s.max(0) as u16); },
        1664 => for i in 0..4 { let s = w_of(&a, i) as i64 - w_of(&b, i) as i64; if s < 0 { sat = true; } set_w(&mut r, i, s.max(0) as u32); },
        1792 => for i in 0..16 { let s = a[i] as i8 as i32 - b[i] as i8 as i32; if !(-128..=127).contains(&s) { sat = true; } r[i] = s.clamp(-128, 127) as i8 as u8; },
        1856 => for i in 0..8 { let s = h_of(&a, i) as i16 as i32 - h_of(&b, i) as i16 as i32; if !(-32768..=32767).contains(&s) { sat = true; } set_h(&mut r, i, s.clamp(-32768, 32767) as i16 as u16); },
        1920 => for i in 0..4 { let s = w_of(&a, i) as i32 as i64 - w_of(&b, i) as i32 as i64; if s < i32::MIN as i64 || s > i32::MAX as i64 { sat = true; } set_w(&mut r, i, s.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32); },

        // ---- min / max / average ----
        2 => for i in 0..16 { r[i] = a[i].max(b[i]); },
        66 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i).max(h_of(&b, i))); },
        130 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i).max(w_of(&b, i))); },
        258 => for i in 0..16 { r[i] = (a[i] as i8).max(b[i] as i8) as u8; },
        322 => for i in 0..8 { set_h(&mut r, i, (h_of(&a, i) as i16).max(h_of(&b, i) as i16) as u16); },
        386 => for i in 0..4 { set_w(&mut r, i, (w_of(&a, i) as i32).max(w_of(&b, i) as i32) as u32); },
        1034 => for i in 0..4 { set_f(&mut r, i, fmax(f_of(&a, i), f_of(&b, i))); },
        514 => for i in 0..16 { r[i] = a[i].min(b[i]); },
        578 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i).min(h_of(&b, i))); },
        642 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i).min(w_of(&b, i))); },
        770 => for i in 0..16 { r[i] = (a[i] as i8).min(b[i] as i8) as u8; },
        834 => for i in 0..8 { set_h(&mut r, i, (h_of(&a, i) as i16).min(h_of(&b, i) as i16) as u16); },
        898 => for i in 0..4 { set_w(&mut r, i, (w_of(&a, i) as i32).min(w_of(&b, i) as i32) as u32); },
        1098 => for i in 0..4 { set_f(&mut r, i, fmin(f_of(&a, i), f_of(&b, i))); },
        1026 => for i in 0..16 { r[i] = ((a[i] as u32 + b[i] as u32 + 1) >> 1) as u8; },
        1090 => for i in 0..8 { set_h(&mut r, i, ((h_of(&a, i) as u32 + h_of(&b, i) as u32 + 1) >> 1) as u16); },
        1154 => for i in 0..4 { set_w(&mut r, i, ((w_of(&a, i) as u64 + w_of(&b, i) as u64 + 1) >> 1) as u32); },
        1282 => for i in 0..16 { r[i] = ((a[i] as i8 as i32 + b[i] as i8 as i32 + 1) >> 1) as i8 as u8; },
        1346 => for i in 0..8 { set_h(&mut r, i, ((h_of(&a, i) as i16 as i32 + h_of(&b, i) as i16 as i32 + 1) >> 1) as i16 as u16); },
        1410 => for i in 0..4 { set_w(&mut r, i, ((w_of(&a, i) as i32 as i64 + w_of(&b, i) as i32 as i64 + 1) >> 1) as i32 as u32); },

        // ---- logical ----
        1028 => for i in 0..16 { r[i] = a[i] & b[i]; },
        1092 => for i in 0..16 { r[i] = a[i] & !b[i]; },
        1156 => for i in 0..16 { r[i] = a[i] | b[i]; },
        1220 => for i in 0..16 { r[i] = a[i] ^ b[i]; },
        1284 => for i in 0..16 { r[i] = !(a[i] | b[i]); },

        // ---- rotate / shift per element ----
        4 => for i in 0..16 { r[i] = a[i].rotate_left((b[i] & 7) as u32); },
        68 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i).rotate_left((h_of(&b, i) & 15) as u32)); },
        132 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i).rotate_left(w_of(&b, i) & 31)); },
        260 => for i in 0..16 { r[i] = a[i] << (b[i] & 7); },
        324 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i) << (h_of(&b, i) & 15)); },
        388 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i) << (w_of(&b, i) & 31)); },
        516 => for i in 0..16 { r[i] = a[i] >> (b[i] & 7); },
        580 => for i in 0..8 { set_h(&mut r, i, h_of(&a, i) >> (h_of(&b, i) & 15)); },
        644 => for i in 0..4 { set_w(&mut r, i, w_of(&a, i) >> (w_of(&b, i) & 31)); },
        772 => for i in 0..16 { r[i] = ((a[i] as i8) >> (b[i] & 7)) as u8; },
        836 => for i in 0..8 { set_h(&mut r, i, ((h_of(&a, i) as i16) >> (h_of(&b, i) & 15)) as u16); },
        900 => for i in 0..4 { set_w(&mut r, i, ((w_of(&a, i) as i32) >> (w_of(&b, i) & 31)) as u32); },
        // whole-register shifts
        452 | 708 => {
            let sh = (b[15] & 7) as u32;
            let v = u128::from_be_bytes(a);
            r = (if xo == 452 { v << sh } else { v >> sh }).to_be_bytes();
        }
        1036 | 1100 => {
            let sh = (((b[15] >> 3) & 0xF) as u32) * 8;
            let v = u128::from_be_bytes(a);
            r = (if xo == 1036 { v << sh } else { v >> sh }).to_be_bytes();
        }

        // ---- multiply even/odd ----
        8 => for i in 0..8 { set_h(&mut r, i, (a[i * 2 + 1] as u16) * (b[i * 2 + 1] as u16)); },
        520 => for i in 0..8 { set_h(&mut r, i, (a[i * 2] as u16) * (b[i * 2] as u16)); },
        264 => for i in 0..8 { set_h(&mut r, i, ((a[i * 2 + 1] as i8 as i16) * (b[i * 2 + 1] as i8 as i16)) as u16); },
        776 => for i in 0..8 { set_h(&mut r, i, ((a[i * 2] as i8 as i16) * (b[i * 2] as i8 as i16)) as u16); },
        72 => for i in 0..4 { set_w(&mut r, i, h_of(&a, i * 2 + 1) as u32 * h_of(&b, i * 2 + 1) as u32); },
        584 => for i in 0..4 { set_w(&mut r, i, h_of(&a, i * 2) as u32 * h_of(&b, i * 2) as u32); },
        328 => for i in 0..4 { set_w(&mut r, i, ((h_of(&a, i * 2 + 1) as i16 as i32) * (h_of(&b, i * 2 + 1) as i16 as i32)) as u32); },
        840 => for i in 0..4 { set_w(&mut r, i, ((h_of(&a, i * 2) as i16 as i32) * (h_of(&b, i * 2) as i16 as i32)) as u32); },

        // ---- sums across ----
        1544 => for i in 0..4 {
            let mut s = w_of(&b, i) as u64;
            for j in 0..4 { s += a[i * 4 + j] as u64; }
            if s > 0xFFFF_FFFF { sat = true; }
            set_w(&mut r, i, s.min(0xFFFF_FFFF) as u32);
        },
        1800 => for i in 0..4 {
            let mut s = w_of(&b, i) as i32 as i64;
            for j in 0..4 { s += a[i * 4 + j] as i8 as i64; }
            if s < i32::MIN as i64 || s > i32::MAX as i64 { sat = true; }
            set_w(&mut r, i, s.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32);
        },
        1608 => for i in 0..4 {
            let mut s = w_of(&b, i) as i32 as i64;
            for j in 0..2 { s += h_of(&a, i * 2 + j) as i16 as i64; }
            if s < i32::MIN as i64 || s > i32::MAX as i64 { sat = true; }
            set_w(&mut r, i, s.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32);
        },
        1672 => for i in 0..2 {
            let s = w_of(&a, i * 2) as i32 as i64 + w_of(&a, i * 2 + 1) as i32 as i64 + w_of(&b, i * 2 + 1) as i32 as i64;
            if s < i32::MIN as i64 || s > i32::MAX as i64 { sat = true; }
            set_w(&mut r, i * 2 + 1, s.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32);
        },
        1928 => {
            let s: i64 = (0..4).map(|i| w_of(&a, i) as i32 as i64).sum::<i64>() + w_of(&b, 3) as i32 as i64;
            if s < i32::MIN as i64 || s > i32::MAX as i64 { sat = true; }
            set_w(&mut r, 3, s.clamp(i32::MIN as i64, i32::MAX as i64) as i32 as u32);
        }

        // ---- merge ----
        12 | 268 => { let o = if xo == 12 { 0 } else { 8 }; for i in 0..8 { r[i * 2] = a[o + i]; r[i * 2 + 1] = b[o + i]; } }
        76 | 332 => { let o = if xo == 76 { 0 } else { 4 }; for i in 0..4 { set_h(&mut r, i * 2, h_of(&a, o + i)); set_h(&mut r, i * 2 + 1, h_of(&b, o + i)); } }
        140 | 396 => { let o = if xo == 140 { 0 } else { 2 }; for i in 0..2 { set_w(&mut r, i * 2, w_of(&a, o + i)); set_w(&mut r, i * 2 + 1, w_of(&b, o + i)); } }

        // ---- pack ----
        14 => for i in 0..16 { let s = if i < 8 { &a } else { &b }; r[i] = h_of(s, i % 8) as u8; },
        78 => for i in 0..8 { let s = if i < 4 { &a } else { &b }; set_h(&mut r, i, w_of(s, i % 4) as u16); },
        142 => for i in 0..16 { let s = if i < 8 { &a } else { &b }; let h = h_of(s, i % 8); if h > 255 { sat = true; } r[i] = h.min(255) as u8; },
        206 => for i in 0..8 { let s = if i < 4 { &a } else { &b }; let w = w_of(s, i % 4); if w > 0xFFFF { sat = true; } set_h(&mut r, i, w.min(0xFFFF) as u16); },
        270 => for i in 0..16 { let s = if i < 8 { &a } else { &b }; let h = h_of(s, i % 8) as i16; if !(0..=255).contains(&h) { sat = true; } r[i] = h.clamp(0, 255) as u8; },
        334 => for i in 0..8 { let s = if i < 4 { &a } else { &b }; let w = w_of(s, i % 4) as i32; if !(0..=65535).contains(&w) { sat = true; } set_h(&mut r, i, w.clamp(0, 65535) as u16); },
        398 => for i in 0..16 { let s = if i < 8 { &a } else { &b }; let h = h_of(s, i % 8) as i16; if !(-128..=127).contains(&h) { sat = true; } r[i] = h.clamp(-128, 127) as i8 as u8; },
        462 => for i in 0..8 { let s = if i < 4 { &a } else { &b }; let w = w_of(s, i % 4) as i32; if !(-32768..=32767).contains(&w) { sat = true; } set_h(&mut r, i, w.clamp(-32768, 32767) as i16 as u16); },
        782 => for i in 0..8 {
            let s = if i < 4 { &a } else { &b };
            let w = w_of(s, i % 4);
            let h = (((w >> 24) & 1) << 15) | ((((w >> 16) & 0xFF) >> 3) << 10) | ((((w >> 8) & 0xFF) >> 3) << 5) | ((w & 0xFF) >> 3);
            set_h(&mut r, i, h as u16);
        },

        // ---- unpack ----
        526 | 654 => { let o = if xo == 526 { 0 } else { 8 }; for i in 0..8 { set_h(&mut r, i, (b[o + i] as i8 as i16) as u16); } }
        590 | 718 => { let o = if xo == 590 { 0 } else { 4 }; for i in 0..4 { set_w(&mut r, i, (h_of(&b, o + i) as i16 as i32) as u32); } }
        846 | 974 => {
            let o = if xo == 846 { 0 } else { 4 };
            for i in 0..4 {
                let h = h_of(&b, o + i) as u32;
                let w = (if (h & 0x8000) != 0 { 0xFFu32 } else { 0 } << 24) | (((h >> 10) & 0x1F) << 16) | (((h >> 5) & 0x1F) << 8) | (h & 0x1F);
                set_w(&mut r, i, w);
            }
        }

        // ---- splat ----
        524 => { let x = b[(ua & 15) as usize]; r = [x; 16]; }
        588 => { let x = h_of(&b, (ua & 7) as usize); for i in 0..8 { set_h(&mut r, i, x); } }
        652 => { let x = w_of(&b, (ua & 3) as usize); for i in 0..4 { set_w(&mut r, i, x); } }
        780 => { let x = (((ua as i32) << 27) >> 27) as i8 as u8; r = [x; 16]; }
        844 => { let x = (((ua as i32) << 27) >> 27) as i16 as u16; for i in 0..8 { set_h(&mut r, i, x); } }
        908 => { let x = (((ua as i32) << 27) >> 27) as u32; for i in 0..4 { set_w(&mut r, i, x); } }

        // ---- floating point ----
        266 => for i in 0..4 { set_f(&mut r, i, 1.0 / f_of(&b, i)); },
        330 => for i in 0..4 { set_f(&mut r, i, 1.0 / f_of(&b, i).sqrt()); },
        394 => for i in 0..4 { set_f(&mut r, i, f_of(&b, i).exp2()); },
        458 => for i in 0..4 { set_f(&mut r, i, f_of(&b, i).log2()); },
        522 => for i in 0..4 { set_f(&mut r, i, round_even(f_of(&b, i))); },
        586 => for i in 0..4 { set_f(&mut r, i, f_of(&b, i).trunc()); },
        650 => for i in 0..4 { set_f(&mut r, i, f_of(&b, i).ceil()); },
        714 => for i in 0..4 { set_f(&mut r, i, f_of(&b, i).floor()); },
        778 => { let s = (1u64 << ua) as f32; for i in 0..4 { set_f(&mut r, i, w_of(&b, i) as f32 / s); } }
        842 => { let s = (1u64 << ua) as f32; for i in 0..4 { set_f(&mut r, i, (w_of(&b, i) as i32) as f32 / s); } }
        906 => {
            let s = (1u64 << ua) as f32;
            for i in 0..4 {
                let x = f_of(&b, i) * s;
                if x.is_nan() { set_w(&mut r, i, 0); continue; }
                if x < 0.0 || x >= 4294967296.0 { sat = true; }
                set_w(&mut r, i, if x <= 0.0 { 0 } else if x >= 4294967295.0 { u32::MAX } else { x as u32 });
            }
        }
        970 => {
            let s = (1u64 << ua) as f32;
            for i in 0..4 {
                let x = f_of(&b, i) * s;
                if x.is_nan() { set_w(&mut r, i, 0); continue; }
                if x < -2147483648.0 || x >= 2147483648.0 { sat = true; }
                set_w(&mut r, i, (x as i32) as u32); // Rust float->int casts saturate
            }
        }

        // ---- VSCR ----
        1540 => { set_w(&mut r, 3, regs.vscr); }
        1604 => { regs.vscr = w_of(&b, 3); return Ok(()); }

        _ => return Err(format!("Unimplemented AltiVec VX xo {} (0x{:03X}) insn=0x{:08X}", xo, xo, insn)),
    }

    if sat { regs.vscr |= 1; }
    regs.vr[vd] = r;
    Ok(())
}
