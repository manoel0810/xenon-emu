// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2022 Ben Vanik.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/register_table.inc               SHADER_CONSTANT_* register indices
//   src/xenia/gpu/xenos.h                          xe_gpu_vertex_fetch_t, xe_gpu_texture_fetch_t, VertexFormat,
//                                                  SignedRepeatingFractionMode
//   src/xenia/gpu/spirv_shader_translator_fetch.cc ProcessVertexFetchInstruction (address, index rounding, bounds,
//                                                  endian swap, unpacking, normalization, exp_adjust)
//
//! The shader environment of the emulated GPU: float/bool/loop constants and fetch constants from the register
//! file, vertex data from guest memory, and textures through a `TextureSampler` (a stub until texture decoding).

use super::shader_interp::{ShaderEnv, Vec4};
use super::ucode::FetchInstruction;
use super::xenos::guest_endian_swap;
use crate::memory::MemoryBus;

/// SHADER_CONSTANT_000_X: float constants, 4 registers each (vertex 0..255, pixel 256..511).
pub const FLOAT_CONSTANTS: u32 = 0x4000;
/// SHADER_CONSTANT_FETCH_00_0: 32 texture fetch constants of 6 dwords, or 96 vertex fetch constants of 2.
pub const FETCH_CONSTANTS: u32 = 0x4800;
/// SHADER_CONSTANT_BOOL_000_031: 256 boolean constants, one bit each.
pub const BOOL_CONSTANTS: u32 = 0x4900;
/// SHADER_CONSTANT_LOOP_00: 32 loop constants.
pub const LOOP_CONSTANTS: u32 = 0x4908;

/// xe_gpu_texture_fetch_t, the fields needed to find and describe a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextureFetchConstant {
    pub words: [u32; 6],
}

impl TextureFetchConstant {
    /// FetchConstantType (2 = texture).
    pub fn fetch_type(&self) -> u32 {
        self.words[0] & 3
    }
    pub fn pitch(&self) -> u32 {
        (self.words[0] >> 22) & 0x1FF
    }
    pub fn tiled(&self) -> bool {
        self.words[0] >> 31 != 0
    }
    /// xenos::TextureFormat.
    pub fn format(&self) -> u32 {
        self.words[1] & 0x3F
    }
    pub fn endianness(&self) -> u32 {
        (self.words[1] >> 6) & 3
    }
    /// Physical address of the base level (base_address << 12).
    pub fn base_address(&self) -> u32 {
        (self.words[1] >> 12) << 12
    }
    /// Width and height of a 2D texture (stored minus one).
    pub fn size_2d(&self) -> (u32, u32) {
        ((self.words[2] & 0x1FFF) + 1, ((self.words[2] >> 13) & 0x1FFF) + 1)
    }
}

/// Texture sampling for tfetch.
pub trait TextureSampler {
    fn sample(&mut self, constant: &TextureFetchConstant, op: &FetchInstruction, coords: Vec4) -> Vec4;
}

/// Stands in for texture decoding (Fase 4.4): every texel is opaque white. Records what was asked.
#[derive(Debug, Default)]
pub struct StubSampler {
    pub fetches: Vec<(TextureFetchConstant, Vec4)>,
}

impl TextureSampler for StubSampler {
    fn sample(&mut self, constant: &TextureFetchConstant, _op: &FetchInstruction, coords: Vec4) -> Vec4 {
        self.fetches.push((*constant, coords));
        [1.0; 4]
    }
}

/// A vertex format: bit offset and width of each component and the words it spans (VertexFormat).
struct Layout {
    words: u32,
    float: bool,
    half: bool,
    components: &'static [(u32, u32, u32)], // (word, bit offset, width)
}

fn layout(format: u32) -> Option<Layout> {
    let packed = |components| Some(Layout { words: 1, float: false, half: false, components });
    Some(match format {
        6 => return packed(&[(0, 0, 8), (0, 8, 8), (0, 16, 8), (0, 24, 8)]),
        7 => return packed(&[(0, 0, 10), (0, 10, 10), (0, 20, 10), (0, 30, 2)]),
        16 => return packed(&[(0, 0, 11), (0, 11, 11), (0, 22, 10)]),
        17 => return packed(&[(0, 0, 10), (0, 10, 11), (0, 21, 11)]),
        25 => return packed(&[(0, 0, 16), (0, 16, 16)]),
        26 => Layout { words: 2, float: false, half: false, components: &[(0, 0, 16), (0, 16, 16), (1, 0, 16), (1, 16, 16)] },
        31 => Layout { words: 1, float: true, half: true, components: &[(0, 0, 16), (0, 16, 16)] },
        32 => Layout { words: 2, float: true, half: true, components: &[(0, 0, 16), (0, 16, 16), (1, 0, 16), (1, 16, 16)] },
        33 => Layout { words: 1, float: false, half: false, components: &[(0, 0, 32)] },
        34 => Layout { words: 2, float: false, half: false, components: &[(0, 0, 32), (1, 0, 32)] },
        35 => Layout { words: 4, float: false, half: false, components: &[(0, 0, 32), (1, 0, 32), (2, 0, 32), (3, 0, 32)] },
        36 => Layout { words: 1, float: true, half: false, components: &[(0, 0, 32)] },
        37 => Layout { words: 2, float: true, half: false, components: &[(0, 0, 32), (1, 0, 32)] },
        38 => Layout { words: 4, float: true, half: false, components: &[(0, 0, 32), (1, 0, 32), (2, 0, 32), (3, 0, 32)] },
        57 => Layout { words: 3, float: true, half: false, components: &[(0, 0, 32), (1, 0, 32), (2, 0, 32)] },
        _ => return None,
    })
}

/// IEEE half to single (GLSL unpackHalf2x16, as Xenia uses; not the extended Xbox 360 range).
pub fn half_to_f32(h: u32) -> f32 {
    let sign = (h >> 15) & 1;
    let exponent = (h >> 10) & 0x1F;
    let mantissa = h & 0x3FF;
    let magnitude = match exponent {
        0 => mantissa as f32 * (1.0 / 16_777_216.0), // 2^-24
        31 => if mantissa == 0 { f32::INFINITY } else { f32::NAN },
        e => f32::from_bits(((e + 112) << 23) | (mantissa << 13)),
    };
    if sign != 0 { -magnitude } else { magnitude }
}

/// Converts the fetched words of one element to the format's components (missing ones are 0).
pub fn unpack_vertex(words: &[u32; 4], format: u32, is_signed: bool, is_integer: bool, signed_rf_no_zero: bool, exp_adjust: i32) -> Vec4 {
    let Some(l) = layout(format) else { return [0.0; 4] };
    let mut out = [0.0f32; 4];
    for (i, &(word, offset, width)) in l.components.iter().enumerate() {
        let w = words[word as usize];
        let bits = if width == 32 { w } else { (w >> offset) & ((1 << width) - 1) };
        out[i] = if l.float {
            if l.half { half_to_f32(bits) } else { f32::from_bits(bits) }
        } else {
            let value = if is_signed { (((bits << (32 - width)) as i32) >> (32 - width)) as f32 } else { bits as f32 };
            if is_integer {
                value
            } else {
                // normalization: signed by 2^(n-1) - 1 (+0.5 in the no-zero mode), unsigned by 2^n - 1
                let scale_inv = if is_signed {
                    ((1u64 << (width - 1)) - 1) as f32 + if signed_rf_no_zero { 0.5 } else { 0.0 }
                } else {
                    ((1u64 << width) - 1) as f32
                };
                let scale = 1.0 / scale_inv;
                let v = value * scale;
                if !is_signed {
                    v
                } else if signed_rf_no_zero {
                    v + 0.5 * scale
                } else {
                    v.max(-1.0)
                }
            }
        };
    }
    if exp_adjust != 0 {
        let factor = 2f32.powi(exp_adjust);
        for v in out.iter_mut().take(l.components.len()) {
            *v *= factor;
        }
    }
    out
}

/// The emulated GPU's shader environment.
pub struct GpuShaderEnv<'a> {
    pub bus: &'a MemoryBus,
    pub sampler: &'a mut dyn TextureSampler,
}

impl GpuShaderEnv<'_> {
    fn reg(&self, index: u32) -> u32 {
        self.bus.gpu_regs.get(index)
    }

    pub fn texture_fetch_constant(&self, index: u32) -> TextureFetchConstant {
        let base = FETCH_CONSTANTS + 6 * (index & 31);
        TextureFetchConstant { words: std::array::from_fn(|i| self.reg(base + i as u32)) }
    }
}

impl ShaderEnv for GpuShaderEnv<'_> {
    fn float_constant(&self, index: u32) -> Vec4 {
        let base = FLOAT_CONSTANTS + 4 * (index & 511);
        std::array::from_fn(|i| f32::from_bits(self.reg(base + i as u32)))
    }

    fn bool_constant(&self, index: u32) -> bool {
        let index = index & 255;
        self.reg(BOOL_CONSTANTS + index / 32) >> (index % 32) & 1 != 0
    }

    fn loop_constant(&self, index: u32) -> u32 {
        self.reg(LOOP_CONSTANTS + (index & 31))
    }

    fn vertex_fetch(&mut self, op: &FetchInstruction, full: &FetchInstruction, index: f32) -> Vec4 {
        // xe_gpu_vertex_fetch_t: base in dwords (bits 2..31 of word 0), endian and size in dwords in word 1
        let constant = FETCH_CONSTANTS + 2 * full.vf_fetch_constant_index();
        let (word0, word1) = (self.reg(constant), self.reg(constant + 1));
        let base = ((word0 >> 2) & 0x07FF_FFFF) as i64;
        let end = base + ((word1 >> 2) & 0x00FF_FFFF) as i64;
        let mut address = base;
        if full.vf_stride() != 0 {
            let index = if full.vf_is_index_rounded() { index + 0.5 } else { index };
            address += index.floor() as i64 * full.vf_stride() as i64;
        }
        let mut words = [0u32; 4];
        let count = layout(op.vf_data_format()).map_or(0, |l| l.words);
        for (i, w) in words.iter_mut().enumerate().take(count as usize) {
            let word_address = address + op.vf_offset() as i64 + i as i64;
            // words at or past the end of the buffer read as 0
            if word_address >= 0 && word_address < end {
                let raw = self.bus.read_u32((word_address as u64) * 4).unwrap_or(0);
                *w = guest_endian_swap(raw, word1 & 3);
            }
        }
        unpack_vertex(&words, op.vf_data_format(), op.vf_is_signed(), !op.vf_is_normalized(), op.vf_signed_rf_mode() == 1, op.vf_exp_adjust())
    }

    fn texture_fetch(&mut self, op: &FetchInstruction, coords: Vec4) -> Vec4 {
        let constant = self.texture_fetch_constant(op.tf_fetch_constant_index());
        self.sampler.sample(&constant, op, coords)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertex_formats_unpack_like_xenia() {
        let f = |w: [u32; 4], fmt, signed, integer| unpack_vertex(&w, fmt, signed, integer, false, 0);
        // FMT_32_32_32_FLOAT
        assert_eq!(f([1.5f32.to_bits(), (-2.0f32).to_bits(), 0.25f32.to_bits(), 0], 57, false, false), [1.5, -2.0, 0.25, 0.0]);
        // FMT_8_8_8_8 unsigned normalized: component 0 in the low byte
        assert_eq!(f([0xFF80_4000, 0, 0, 0], 6, false, false), [0.0, 64.0 / 255.0, 128.0 / 255.0, 1.0]);
        // signed normalized, -128 clamps to -1; integer keeps the value
        assert_eq!(f([0x8081_7F00, 0, 0, 0], 6, true, false), [0.0, 1.0, -1.0, -1.0]);
        assert_eq!(f([0x0000_FF05, 0, 0, 0], 6, true, true), [5.0, -1.0, 0.0, 0.0]);
        // FMT_16_16_FLOAT: 1.0 = 0x3C00, -2.0 = 0xC000
        assert_eq!(f([0xC000_3C00, 0, 0, 0], 31, false, false), [1.0, -2.0, 0.0, 0.0]);
        // FMT_2_10_10_10 unsigned normalized
        assert_eq!(f([(3 << 30) | (1023 << 20) | 0, 0, 0, 0], 7, false, false), [0.0, 0.0, 1.0, 1.0]);
        // no-zero signed mode: (v + 0.5) / (2^(n-1) - 0.5)
        assert_eq!(unpack_vertex(&[0x0000_0000, 0, 0, 0], 6, true, false, true, 0)[0], 0.5 * (1.0 / 127.5));
        // exp_adjust scales by 2^n
        assert_eq!(unpack_vertex(&[1.0f32.to_bits(), 0, 0, 0], 36, false, false, false, 3)[0], 8.0);
        assert_eq!(f([0; 4], 63, false, false), [0.0; 4], "unknown format");
        assert!(half_to_f32(0x7C01).is_nan());
        assert_eq!(half_to_f32(0x0001), 2f32.powi(-24));
    }

    #[test]
    fn constants_come_from_the_register_file() {
        let mut bus = MemoryBus::new();
        let mut sampler = StubSampler::default();
        for (i, v) in [1.0f32, 2.0, 3.0, 4.0].iter().enumerate() {
            bus.gpu_regs.set(FLOAT_CONSTANTS + 4 * 257 + i as u32, v.to_bits());
        }
        bus.gpu_regs.set(BOOL_CONSTANTS + 1, 1 << 3);
        bus.gpu_regs.set(LOOP_CONSTANTS + 2, 0x0001_0203);
        let env = GpuShaderEnv { bus: &bus, sampler: &mut sampler };
        assert_eq!(env.float_constant(257), [1.0, 2.0, 3.0, 4.0]);
        assert!(env.bool_constant(35) && !env.bool_constant(34));
        assert_eq!(env.loop_constant(2), 0x0001_0203);
    }

    #[test]
    fn vertex_fetch_reads_guest_memory_with_stride_offset_and_bounds() {
        let mut bus = MemoryBus::new();
        let mut sampler = StubSampler::default();
        // vertex fetch constant 1 (registers 0x4802/0x4803): base 0x0100_0000, 8 dwords, endian 2 (k8in32)
        bus.gpu_regs.set(FETCH_CONSTANTS + 2, 0x0100_0000 | 3);
        bus.gpu_regs.set(FETCH_CONSTANTS + 3, (8 << 2) | 2);
        for i in 0..10u32 {
            bus.write_u32(0x0100_0000 + 4 * i as u64, (i as f32).to_bits()).unwrap();
        }
        // vfetch_full constant 1 (index 0, sel 1), FMT_32_32_FLOAT, stride 3, offset 1
        let op = FetchInstruction([(1 << 19) | (1 << 25), 37 << 16, 3 | (1 << 8)]);
        let mut env = GpuShaderEnv { bus: &bus, sampler: &mut sampler };
        assert_eq!(env.vertex_fetch(&op, &op, 1.0), [4.0, 5.0, 0.0, 0.0]);
        // index 2.6 floors to 2 (6 + 1); with index rounding it is 3, and dword 10 is past the 8-dword buffer
        assert_eq!(env.vertex_fetch(&op, &op, 2.6), [7.0, 0.0, 0.0, 0.0], "dword 8 is out of bounds");
        let rounded = FetchInstruction([op.0[0], op.0[1] | (1 << 15), op.0[2]]);
        assert_eq!(env.vertex_fetch(&rounded, &rounded, 0.6), [4.0, 5.0, 0.0, 0.0]);
        // endian 0 (no swap): the dwords read are byte-swapped
        bus.gpu_regs.set(FETCH_CONSTANTS + 3, 8 << 2);
        let mut env = GpuShaderEnv { bus: &bus, sampler: &mut sampler };
        assert_eq!(env.vertex_fetch(&op, &op, 0.0)[0], f32::from_bits(1.0f32.to_bits().swap_bytes()));
    }

    #[test]
    fn texture_fetch_goes_to_the_sampler_with_its_constant() {
        let mut bus = MemoryBus::new();
        // texture fetch constant 3: 2D, tiled, format 6 (k_8_8_8_8), base 0x0012_3000, 1280x720
        let base = FETCH_CONSTANTS + 18;
        bus.gpu_regs.set(base, 2 | (1 << 31) | (40 << 22));
        bus.gpu_regs.set(base + 1, 6 | (2 << 6) | (0x123 << 12));
        bus.gpu_regs.set(base + 2, 1279 | (719 << 13));
        let mut sampler = StubSampler::default();
        let op = FetchInstruction([1 | (3 << 20), 0, 1 << 14]);
        let mut env = GpuShaderEnv { bus: &bus, sampler: &mut sampler };
        assert_eq!(env.texture_fetch(&op, [0.5, 0.25, 0.0, 0.0]), [1.0; 4]);
        let (c, coords) = sampler.fetches[0];
        assert_eq!((c.fetch_type(), c.tiled(), c.pitch(), c.format(), c.endianness()), (2, true, 40, 6, 2));
        assert_eq!((c.base_address(), c.size_2d(), coords), (0x0012_3000, (1280, 720), [0.5, 0.25, 0.0, 0.0]));
    }
}
