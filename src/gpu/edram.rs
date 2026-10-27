// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/xenos.h              EDRAM tiles (kEdramTileWidthSamples, kEdramTileHeightSamples, kEdramTileCount),
//                                      GetSurfacePitchTiles, EdramMode, CopyCommand, CopySampleSelect,
//                                      ColorRenderTargetFormat, DepthRenderTargetFormat, BlendFactor, BlendOp,
//                                      CompareFunction, StencilOp; depth tiles with their 40-sample halves swapped
//   src/xenia/gpu/registers.h          RB_SURFACE_INFO, RB_COLOR_INFO, RB_DEPTH_INFO, RB_DEPTHCONTROL, RB_BLENDCONTROL,
//                                      RB_COLOR_MASK, RB_COLORCONTROL, RB_COPY_*, PA_SC_WINDOW_*, PA_SC_SCREEN_SCISSOR_*
//   src/xenia/gpu/register_table.inc   register indices
//   src/xenia/gpu/render_target_cache.h  depth in bits 8..31 of a D24S8 sample
//   src/xenia/gpu/draw_util.cc         GetScissor, GetResolveInfo (rectangle from vertex fetch 0, 16.8 fixed point,
//                                      window offset, alignment, EDRAM base/pitch in tiles, destination macro phase)
//
//! The Xenos EDRAM and render backend: 10 MiB of 32-bit samples in 2048 tiles of 80x16 samples, render targets
//! addressed by base and pitch in tiles (so surfaces alias as on the console), depth/stencil and color writes, and
//! resolves (copy draws) from EDRAM to guest memory.
//!
//! Deviations from the console (documented): coverage is evaluated once per pixel, at its center, and written to all
//! of the pixel's samples (no MSAA edge antialiasing); sample `s` of a 2x pixel (x, y) is (x, 2y + s), of a 4x pixel
//! (2x + (s & 1), 2y + (s >> 1)). Only k_8_8_8_8(_GAMMA) color, D24S8 depth and k_8_8_8_8 resolve destinations are
//! implemented; other formats are reported once and not written.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use crate::gpu::texture::tiled_offset_2d;
use crate::gpu::xenos::guest_endian_swap;
use crate::memory::MemoryBus;

pub const EDRAM_TILE_WIDTH_SAMPLES: u32 = 80;
pub const EDRAM_TILE_HEIGHT_SAMPLES: u32 = 16;
pub const EDRAM_TILE_COUNT: u32 = 2048;
const TILE_SAMPLES: usize = (EDRAM_TILE_WIDTH_SAMPLES * EDRAM_TILE_HEIGHT_SAMPLES) as usize;

pub const RB_SURFACE_INFO: u32 = 0x2000;
pub const RB_COLOR_INFO: u32 = 0x2001;
pub const RB_DEPTH_INFO: u32 = 0x2002;
/// RB_COLOR_INFO of render targets 0..3.
pub const RB_COLOR_INFOS: [u32; 4] = [0x2001, 0x2003, 0x2004, 0x2005];
pub const PA_SC_SCREEN_SCISSOR_TL: u32 = 0x200E;
pub const PA_SC_SCREEN_SCISSOR_BR: u32 = 0x200F;
pub const PA_SC_WINDOW_OFFSET: u32 = 0x2080;
pub const PA_SC_WINDOW_SCISSOR_TL: u32 = 0x2081;
pub const PA_SC_WINDOW_SCISSOR_BR: u32 = 0x2082;
pub const RB_COLOR_MASK: u32 = 0x2104;
pub const RB_BLEND_RED: u32 = 0x2105;
pub const RB_STENCILREFMASK_BF: u32 = 0x210C;
pub const RB_STENCILREFMASK: u32 = 0x210D;
pub const RB_ALPHA_REF: u32 = 0x210E;
pub const RB_DEPTHCONTROL: u32 = 0x2200;
/// RB_BLENDCONTROL of render targets 0..3.
pub const RB_BLENDCONTROLS: [u32; 4] = [0x2201, 0x2209, 0x220A, 0x220B];
pub const RB_COLORCONTROL: u32 = 0x2202;
pub const PA_SU_SC_MODE_CNTL: u32 = 0x2205;
pub const RB_MODECONTROL: u32 = 0x2208;
pub const PA_SU_VTX_CNTL: u32 = 0x2302;
pub const RB_COPY_CONTROL: u32 = 0x2318;
pub const RB_COPY_DEST_BASE: u32 = 0x2319;
pub const RB_COPY_DEST_PITCH: u32 = 0x231A;
pub const RB_COPY_DEST_INFO: u32 = 0x231B;
pub const RB_DEPTH_CLEAR: u32 = 0x231D;
pub const RB_COLOR_CLEAR: u32 = 0x231E;
/// SHADER_CONSTANT_FETCH_00_0: vertex fetch constant 0 (2 dwords).
const FETCH_00: u32 = 0x4800;

/// EdramMode (RB_MODECONTROL.edram_mode).
pub const EDRAM_MODE_COLOR_DEPTH: u32 = 4;
pub const EDRAM_MODE_DEPTH_ONLY: u32 = 5;
pub const EDRAM_MODE_COPY: u32 = 6;

/// ColorRenderTargetFormat k_8_8_8_8 / k_8_8_8_8_GAMMA; DepthRenderTargetFormat kD24S8.
const COLOR_8_8_8_8: u32 = 0;
const COLOR_8_8_8_8_GAMMA: u32 = 1;
const DEPTH_D24S8: u32 = 0;
/// TextureFormat k_8_8_8_8 (resolve destination).
const TEXTURE_8_8_8_8: u32 = 6;
/// CopyCommand kNull: clear only.
const COPY_COMMAND_NULL: u32 = 3;
/// kResolveAlignmentPixels, kMaxResolveSize.
const RESOLVE_ALIGNMENT: i32 = 8;
const MAX_RESOLVE_SIZE: i32 = (1 << 14) - 8;

/// Sign-extends the low `bits` bits of `v`.
fn signed(v: u32, bits: u32) -> i32 {
    ((v << (32 - bits)) as i32) >> (32 - bits)
}

/// xenos::GetSurfacePitchTiles.
pub fn surface_pitch_tiles(pitch_pixels: u32, msaa_samples: u32, is_64bpp: bool) -> u32 {
    let pitch_samples = if msaa_samples >= 2 { pitch_pixels << 1 } else { pitch_pixels };
    let tiles = pitch_samples.div_ceil(EDRAM_TILE_WIDTH_SAMPLES);
    if is_64bpp { tiles << 1 } else { tiles }
}

/// A render target or resolve source in EDRAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Surface {
    /// First tile (RB_COLOR_INFO.color_base / RB_DEPTH_INFO.depth_base, 12 bits).
    pub base: u32,
    pub pitch_tiles: u32,
    /// MsaaSamples: 0 = 1x, 1 = 2x, 2 = 4x.
    pub msaa: u32,
    pub depth: bool,
}

impl Surface {
    /// Samples per pixel horizontally and vertically.
    pub fn sample_grid(&self) -> (u32, u32) {
        match self.msaa {
            0 => (1, 1),
            1 => (1, 2),
            _ => (2, 2),
        }
    }

    pub fn samples_per_pixel(&self) -> u32 {
        let (w, h) = self.sample_grid();
        w * h
    }

    /// Index in `Edram::samples` of sample (sx, sy); tiles wrap around the 2048 of the EDRAM, and depth tiles have
    /// their two 40-sample halves swapped.
    pub fn sample_index(&self, sx: u32, sy: u32) -> usize {
        let tile = (self.base + (sy / EDRAM_TILE_HEIGHT_SAMPLES) * self.pitch_tiles + sx / EDRAM_TILE_WIDTH_SAMPLES) % EDRAM_TILE_COUNT;
        let mut column = sx % EDRAM_TILE_WIDTH_SAMPLES;
        if self.depth {
            column = (column + EDRAM_TILE_WIDTH_SAMPLES / 2) % EDRAM_TILE_WIDTH_SAMPLES;
        }
        tile as usize * TILE_SAMPLES + ((sy % EDRAM_TILE_HEIGHT_SAMPLES) * EDRAM_TILE_WIDTH_SAMPLES + column) as usize
    }

    /// Index of sample `s` of pixel (x, y).
    pub fn pixel_sample_index(&self, x: u32, y: u32, s: u32) -> usize {
        let (w, h) = self.sample_grid();
        self.sample_index(x * w + (s % w), y * h + (s / w))
    }
}

/// k_8_8_8_8 sample: R in bits 0..7, then G, B, A.
pub fn pack_8888(c: [f32; 4]) -> u32 {
    c.iter().enumerate().fold(0, |v, (i, x)| v | (((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * i)))
}

pub fn unpack_8888(v: u32) -> [f32; 4] {
    std::array::from_fn(|i| ((v >> (8 * i)) & 0xFF) as f32 / 255.0)
}

/// CompareFunction.
fn compare(func: u32, a: u32, b: u32) -> bool {
    match func & 7 {
        0 => false,
        1 => a < b,
        2 => a == b,
        3 => a <= b,
        4 => a > b,
        5 => a != b,
        6 => a >= b,
        _ => true,
    }
}

/// StencilOp applied to `s` (8 bits).
fn stencil_op(op: u32, s: u32, reference: u32) -> u32 {
    match op & 7 {
        0 => s,
        1 => 0,
        2 => reference,
        3 => (s + 1).min(0xFF),
        4 => s.saturating_sub(1),
        5 => !s & 0xFF,
        6 => (s + 1) & 0xFF,
        _ => s.wrapping_sub(1) & 0xFF,
    }
}

/// RB_DEPTHCONTROL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthControl(pub u32);

impl DepthControl {
    pub fn stencil_enable(self) -> bool {
        self.0 & 1 != 0
    }
    pub fn z_enable(self) -> bool {
        self.0 & 2 != 0
    }
    pub fn z_write_enable(self) -> bool {
        self.0 & 4 != 0
    }
    pub fn zfunc(self) -> u32 {
        (self.0 >> 4) & 7
    }
    pub fn backface_enable(self) -> bool {
        self.0 & 0x80 != 0
    }
    /// (func, fail, zpass, zfail) for the front or back face.
    pub fn stencil(self, back: bool) -> (u32, u32, u32, u32) {
        let shift = if back && self.backface_enable() { 20 } else { 8 };
        let f = |i: u32| (self.0 >> (shift + 3 * i)) & 7;
        (f(0), f(1), f(2), f(3))
    }
}

/// RB_BLENDCONTROL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendControl(pub u32);

impl BlendControl {
    /// BlendFactor for the destination color `dst`, source `src` and blend constant `constant`.
    fn factor(f: u32, src: [f32; 4], dst: [f32; 4], constant: [f32; 4], component: usize) -> f32 {
        let c = component;
        match f {
            0 => 0.0,
            1 => 1.0,
            4 => src[c],
            5 => 1.0 - src[c],
            6 => src[3],
            7 => 1.0 - src[3],
            8 => dst[c],
            9 => 1.0 - dst[c],
            10 => dst[3],
            11 => 1.0 - dst[3],
            12 => constant[c],
            13 => 1.0 - constant[c],
            14 => constant[3],
            15 => 1.0 - constant[3],
            // kSrcAlphaSaturate
            16 => if c == 3 { 1.0 } else { src[3].min(1.0 - dst[3]) },
            _ => 0.0,
        }
    }

    fn combine(op: u32, s: f32, fs: f32, d: f32, fd: f32) -> f32 {
        match op {
            1 => s * fs - d * fd,
            2 => s.min(d),
            3 => s.max(d),
            4 => d * fd - s * fs,
            _ => s * fs + d * fd,
        }
    }

    pub fn blend(self, src: [f32; 4], dst: [f32; 4], constant: [f32; 4]) -> [f32; 4] {
        let v = self.0;
        let (color_src, color_op, color_dst) = (v & 0x1F, (v >> 5) & 7, (v >> 8) & 0x1F);
        let (alpha_src, alpha_op, alpha_dst) = ((v >> 16) & 0x1F, (v >> 21) & 7, (v >> 24) & 0x1F);
        std::array::from_fn(|c| {
            let (fs, op, fd) = if c == 3 { (alpha_src, alpha_op, alpha_dst) } else { (color_src, color_op, color_dst) };
            Self::combine(op, src[c], Self::factor(fs, src, dst, constant, c), dst[c], Self::factor(fd, src, dst, constant, c))
        })
    }
}

/// Scissor rectangle in pixels, [x0, x1) x [y0, y1) (draw_util GetScissor, not clamped to the surface pitch).
pub fn scissor(bus: &MemoryBus) -> (i32, i32, i32, i32) {
    let r = |i| bus.gpu_regs.get(i);
    let (wtl, wbr) = (r(PA_SC_WINDOW_SCISSOR_TL), r(PA_SC_WINDOW_SCISSOR_BR));
    let (mut x0, mut y0) = ((wtl & 0x3FFF) as i32, ((wtl >> 16) & 0x3FFF) as i32);
    let (mut x1, mut y1) = ((wbr & 0x3FFF) as i32, ((wbr >> 16) & 0x3FFF) as i32);
    if wtl >> 31 == 0 {
        let offset = r(PA_SC_WINDOW_OFFSET);
        let (ox, oy) = (signed(offset, 15), signed(offset >> 16, 15));
        x0 += ox;
        y0 += oy;
        x1 += ox;
        y1 += oy;
    }
    let (stl, sbr) = (r(PA_SC_SCREEN_SCISSOR_TL), r(PA_SC_SCREEN_SCISSOR_BR));
    x0 = x0.max(signed(stl, 15));
    y0 = y0.max(signed(stl >> 16, 15));
    x1 = x1.min(signed(sbr, 15));
    y1 = y1.min(signed(sbr >> 16, 15));
    let (x0, y0) = (x0.max(0), y0.max(0));
    (x0, y0, x1.max(x0), y1.max(y0))
}

/// ui::FloatToD3D11Fixed16p8: round to nearest even in 16.8 fixed point, saturating.
fn to_fixed_16p8(f: f32) -> i32 {
    if f.is_nan() || f.abs() < 1.0 / 512.0 {
        0
    } else {
        (f * 256.0).round_ties_even().clamp(-(1 << 23) as f32, ((1 << 23) - 1) as f32) as i32
    }
}

/// The render state of one draw, read from the registers.
#[derive(Debug, Clone)]
pub struct RenderState {
    pub mode: u32,
    /// Bound color render targets (index, surface, format).
    pub colors: Vec<(usize, Surface, u32)>,
    pub depth: Option<(Surface, u32)>,
    pub depth_control: DepthControl,
    pub color_mask: u32,
    pub blend: [BlendControl; 4],
    pub blend_constant: [f32; 4],
    /// (ref, mask, write mask) front and back.
    pub stencil_ref: [(u32, u32, u32); 2],
    /// RB_COLORCONTROL alpha test: (enabled, func), RB_ALPHA_REF.
    pub alpha_test: (bool, u32),
    pub alpha_ref: f32,
    pub scissor: (i32, i32, i32, i32),
    pub msaa: u32,
}

impl RenderState {
    /// None when the draw writes nothing (no-op mode, no surface pitch, nothing bound).
    pub fn from_registers(bus: &MemoryBus) -> Option<Self> {
        let r = |i| bus.gpu_regs.get(i);
        let mode = r(RB_MODECONTROL) & 7;
        if mode != EDRAM_MODE_COLOR_DEPTH && mode != EDRAM_MODE_DEPTH_ONLY {
            return None;
        }
        let surface_info = r(RB_SURFACE_INFO);
        let (pitch, msaa) = (surface_info & 0x3FFF, (surface_info >> 16) & 3);
        if pitch == 0 {
            return None;
        }
        let color_mask = r(RB_COLOR_MASK);
        let mut colors = Vec::new();
        if mode == EDRAM_MODE_COLOR_DEPTH {
            for (i, &reg) in RB_COLOR_INFOS.iter().enumerate() {
                if (color_mask >> (4 * i)) & 0xF != 0 {
                    let info = r(reg);
                    let format = (info >> 16) & 0xF;
                    let is_64bpp = matches!(format, 5 | 7 | 15);
                    colors.push((i, Surface { base: info & 0xFFF, pitch_tiles: surface_pitch_tiles(pitch, msaa, is_64bpp), msaa, depth: false }, format));
                }
            }
        }
        let depth_control = DepthControl(r(RB_DEPTHCONTROL));
        let depth = (depth_control.z_enable() || depth_control.stencil_enable()).then(|| {
            let info = r(RB_DEPTH_INFO);
            (Surface { base: info & 0xFFF, pitch_tiles: surface_pitch_tiles(pitch, msaa, false), msaa, depth: true }, (info >> 16) & 1)
        });
        if colors.is_empty() && depth.is_none() {
            return None;
        }
        let refmask = |v: u32| (v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF);
        let color_control = r(RB_COLORCONTROL);
        Some(Self {
            mode,
            colors,
            depth,
            depth_control,
            color_mask,
            blend: RB_BLENDCONTROLS.map(|reg| BlendControl(r(reg))),
            blend_constant: std::array::from_fn(|i| f32::from_bits(r(RB_BLEND_RED + i as u32))),
            stencil_ref: [refmask(r(RB_STENCILREFMASK)), refmask(r(RB_STENCILREFMASK_BF))],
            alpha_test: (color_control & 8 != 0, color_control & 7),
            alpha_ref: f32::from_bits(r(RB_ALPHA_REF)),
            scissor: scissor(bus),
            msaa,
        })
    }
}


/// The EDRAM: 2048 tiles of 80x16 32-bit samples.
pub struct Edram {
    pub samples: Vec<u32>,
    /// Unsupported formats already reported.
    warned: HashSet<String>,
}

impl Default for Edram {
    fn default() -> Self {
        Self::new()
    }
}

impl Edram {
    pub fn new() -> Self {
        Self { samples: vec![0; EDRAM_TILE_COUNT as usize * TILE_SAMPLES], warned: HashSet::new() }
    }

    pub fn warn_once(&mut self, what: String) {
        if self.warned.insert(what.clone()) {
            println!("  [EDRAM] {} (not emulated)", what);
        }
    }

    /// The samples as cells that rasterizer threads working on disjoint pixels can share.
    pub fn cells(&mut self) -> &[AtomicU32] {
        // SAFETY: AtomicU32 has the layout of u32, and the exclusive borrow keeps any other access out
        unsafe { &*(self.samples.as_mut_slice() as *mut [u32] as *const [AtomicU32]) }
    }

    /// Reports the formats of a draw's targets that are not emulated (their writes are skipped).
    pub fn check_formats(&mut self, state: &RenderState) {
        if let Some((_, format)) = state.depth {
            if format != DEPTH_D24S8 {
                self.warn_once(format!("depth format {}", format));
            }
        }
        for &(_, _, format) in &state.colors {
            if format == COLOR_8_8_8_8_GAMMA {
                self.warn_once("gamma conversion of k_8_8_8_8_GAMMA".to_string());
            } else if format != COLOR_8_8_8_8 {
                self.warn_once(format!("color format {}", format));
            }
        }
    }
}

/// Depth/stencil test and update of every sample of pixel (x, y) of the depth surface, with D24S8 depth `z` in [0, 1].
/// Returns whether the fragment passes. Unsupported depth formats pass without a write.
pub fn depth_stencil(samples: &[AtomicU32], state: &RenderState, x: u32, y: u32, z: f32, back: bool) -> bool {
    let Some((surface, format)) = state.depth else { return true };
    if format != DEPTH_D24S8 {
        return true;
    }
    let dc = state.depth_control;
    let z24 = (z.clamp(0.0, 1.0) * 16_777_215.0).round() as u32;
    let (reference, mask, write_mask) = state.stencil_ref[back as usize];
    let (func, fail, zpass, zfail) = dc.stencil(back);
    // coverage is per pixel: every sample sees the same fragment
    let mut passed = false;
    for s in 0..surface.samples_per_pixel() {
        let i = surface.pixel_sample_index(x, y, s);
        let stored = samples[i].load(Relaxed);
        let (stored_z, stored_s) = (stored >> 8, stored & 0xFF);
        let stencil_pass = !dc.stencil_enable() || compare(func, reference & mask, stored_s & mask);
        let depth_pass = !dc.z_enable() || compare(dc.zfunc(), z24, stored_z);
        let mut new_s = stored_s;
        if dc.stencil_enable() {
            let op = if !stencil_pass { fail } else if !depth_pass { zfail } else { zpass };
            new_s = (stored_s & !write_mask) | (stencil_op(op, stored_s, reference) & write_mask);
        }
        let new_z = if stencil_pass && depth_pass && dc.z_enable() && dc.z_write_enable() { z24 } else { stored_z };
        samples[i].store((new_z << 8) | new_s, Relaxed);
        passed |= stencil_pass && depth_pass;
    }
    passed
}

/// Blends and writes the pixel shader's color exports to every sample of pixel (x, y) of each bound target.
pub fn write_colors(samples: &[AtomicU32], state: &RenderState, x: u32, y: u32, colors: &[Option<[f32; 4]>; 4]) {
    for &(i, surface, format) in &state.colors {
        let Some(src) = colors[i] else { continue };
        if format != COLOR_8_8_8_8 && format != COLOR_8_8_8_8_GAMMA {
            continue;
        }
        let mask = (state.color_mask >> (4 * i)) & 0xF;
        let keep: u32 = (0..4).filter(|c| mask & (1 << c) == 0).map(|c| 0xFFu32 << (8 * c)).sum();
        for s in 0..surface.samples_per_pixel() {
            let index = surface.pixel_sample_index(x, y, s);
            let dst = samples[index].load(Relaxed);
            let blended = pack_8888(state.blend[i].blend(src, unpack_8888(dst), state.blend_constant));
            samples[index].store((blended & !keep) | (dst & keep), Relaxed);
        }
    }
}

impl RenderState {
    /// Tiles of the EDRAM that the targets touch for pixel rows y0..=y1, as (first tile, count) modulo 2048.
    fn tile_spans(&self, y0: u32, y1: u32) -> Vec<(u32, u32)> {
        let surfaces = self.colors.iter().map(|&(_, s, _)| s).chain(self.depth.map(|(s, _)| s));
        surfaces
            .map(|s| {
                let (_, h) = s.sample_grid();
                let (r0, r1) = (y0 * h / EDRAM_TILE_HEIGHT_SAMPLES, ((y1 + 1) * h - 1) / EDRAM_TILE_HEIGHT_SAMPLES);
                ((s.base + r0 * s.pitch_tiles) % EDRAM_TILE_COUNT, (r1 - r0 + 1) * s.pitch_tiles)
            })
            .collect()
    }

    /// Whether distinct pixels of rows y0..=y1 never share a sample: each target spans at most the EDRAM, and the
    /// targets do not overlap one another (rasterizer threads may then work on disjoint rows).
    pub fn pixels_are_disjoint(&self, y0: u32, y1: u32) -> bool {
        let spans = self.tile_spans(y0, y1);
        if spans.iter().map(|&(_, n)| n as u64).sum::<u64>() > EDRAM_TILE_COUNT as u64 {
            return false;
        }
        let overlaps = |(a, n): (u32, u32), (b, m): (u32, u32)| {
            let d = (b + EDRAM_TILE_COUNT - a) % EDRAM_TILE_COUNT;
            d < n || (EDRAM_TILE_COUNT - d) % EDRAM_TILE_COUNT < m
        };
        spans.iter().enumerate().all(|(i, &a)| spans[i + 1..].iter().all(|&b| !overlaps(a, b)))
    }
}

impl Edram {
    /// RB_MODECONTROL = kCopy (Xenia IssueCopy): resolves the rectangle given by vertex fetch 0 from the selected
    /// EDRAM surface to RB_COPY_DEST_BASE, then applies the requested clears. Returns the resolved rectangle.
    pub fn resolve(&mut self, bus: &mut MemoryBus) -> Option<(i32, i32, i32, i32)> {
        let r = |i| bus.gpu_regs.get(i);
        let copy_control = r(RB_COPY_CONTROL);
        let src_select = copy_control & 7;
        let sample_select = (copy_control >> 4) & 7;
        let color_clear = copy_control & 0x100 != 0;
        let depth_clear = copy_control & 0x200 != 0;
        let command = (copy_control >> 20) & 3;

        // rectangle: three x/y float pairs in vertex fetch 0, written by the CPU
        let (w0, w1) = (r(FETCH_00), r(FETCH_00 + 1));
        if w0 & 3 != 3 || (w1 >> 2) & 0xFF_FFFF != 6 {
            self.warn_once(format!("resolve without a 3-vertex rectangle in vertex fetch 0 ({:08X} {:08X})", w0, w1));
            return None;
        }
        let half_pixel = if r(PA_SU_VTX_CNTL) & 1 == 0 { 0.5 } else { 0.0 };
        let base = ((w0 >> 2) & 0x07FF_FFFF) as u64 * 4;
        let fixed: Vec<i32> = (0..6)
            .map(|i| to_fixed_16p8(f32::from_bits(guest_endian_swap(bus.read_u32(base + 4 * i).unwrap_or(0), w1 & 3)) + half_pixel))
            .collect();
        let xs = [fixed[0], fixed[2], fixed[4]];
        let ys = [fixed[1], fixed[3], fixed[5]];
        let (mut x0, mut y0) = ((xs.iter().min().unwrap() + 127) >> 8, (ys.iter().min().unwrap() + 127) >> 8);
        let (mut x1, mut y1) = ((xs.iter().max().unwrap() + 127) >> 8, (ys.iter().max().unwrap() + 127) >> 8);
        if r(PA_SU_SC_MODE_CNTL) & (1 << 16) != 0 {
            let offset = r(PA_SC_WINDOW_OFFSET);
            let (ox, oy) = (signed(offset, 15), signed(offset >> 16, 15));
            x0 += ox;
            x1 += ox;
            y0 += oy;
            y1 += oy;
        }
        let (sx0, sy0, sx1, sy1) = scissor(bus);
        x0 = x0.clamp(sx0, sx1);
        x1 = x1.clamp(sx0, sx1);
        y0 = y0.clamp(sy0, sy1);
        y1 = y1.clamp(sy0, sy1);
        let surface_info = r(RB_SURFACE_INFO);
        let (pitch, msaa) = ((surface_info & 0x3FFF) as i32, (surface_info >> 16) & 3);
        let align_up = |v: i32| (v + RESOLVE_ALIGNMENT - 1) & !(RESOLVE_ALIGNMENT - 1);
        x0 &= !(RESOLVE_ALIGNMENT - 1);
        y0 &= !(RESOLVE_ALIGNMENT - 1);
        x1 = align_up(x1).min(align_up(pitch));
        y1 = align_up(y1).min(y0 + MAX_RESOLVE_SIZE);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }

        let color_index = if src_select < 4 { src_select as usize } else { 0 };
        let color_info = r(RB_COLOR_INFOS[color_index]);
        let color_format = (color_info >> 16) & 0xF;
        let color = Surface {
            base: color_info & 0xFFF,
            pitch_tiles: surface_pitch_tiles(pitch as u32, msaa, matches!(color_format, 5 | 7 | 15)),
            msaa,
            depth: false,
        };
        let depth_info = r(RB_DEPTH_INFO);
        let depth = Surface { base: depth_info & 0xFFF, pitch_tiles: surface_pitch_tiles(pitch as u32, msaa, false), msaa, depth: true };

        let (color_clear_value, depth_clear_value) = (r(RB_COLOR_CLEAR), r(RB_DEPTH_CLEAR));
        if command != COPY_COMMAND_NULL {
            if src_select >= 4 {
                self.warn_once("depth resolve".to_string());
            } else if color_format != COLOR_8_8_8_8 && color_format != COLOR_8_8_8_8_GAMMA {
                self.warn_once(format!("resolve from color format {}", color_format));
            } else {
                self.copy_color(bus, &color, sample_select, (x0, y0, x1, y1));
            }
        }

        // clears: the resolved rectangle of the color source and of the depth surface
        let (gw, gh) = color.sample_grid();
        let samples = |s: &Surface, value: u32, edram: &mut Edram| {
            for y in (y0 as u32 * gh)..(y1 as u32 * gh) {
                for x in (x0 as u32 * gw)..(x1 as u32 * gw) {
                    let i = s.sample_index(x, y);
                    edram.samples[i] = value;
                }
            }
        };
        if color_clear {
            if matches!(color_format, 5 | 7 | 15) {
                self.warn_once(format!("64-bpp clear of color format {}", color_format));
            } else {
                samples(&color, color_clear_value, self);
            }
        }
        if depth_clear {
            samples(&depth, depth_clear_value, self);
        }
        Some((x0, y0, x1, y1))
    }

    /// Copies a k_8_8_8_8 color rectangle to the RB_COPY_DEST_* texture, averaging the selected samples.
    fn copy_color(&mut self, bus: &mut MemoryBus, color: &Surface, sample_select: u32, (x0, y0, x1, y1): (i32, i32, i32, i32)) {
        let r = |i| bus.gpu_regs.get(i);
        let dest_info = r(RB_COPY_DEST_INFO);
        let endian = dest_info & 7;
        let dest_format = (dest_info >> 7) & 0x3F;
        let exp_bias = signed(dest_info >> 16, 6);
        let swap_rb = dest_info & (1 << 24) != 0;
        if dest_info & 8 != 0 {
            self.warn_once("resolve to a 3D/array destination".to_string());
            return;
        }
        if dest_format != TEXTURE_8_8_8_8 {
            self.warn_once(format!("resolve destination format {}", dest_format));
            return;
        }
        if endian > 3 {
            self.warn_once(format!("resolve destination endian {}", endian));
        }
        // CopySampleSelect: k0..k3, k01, k23, k0123; samples that the surface does not have fall back to the first ones
        let available = color.samples_per_pixel();
        let selected: Vec<u32> = match sample_select {
            0..=3 => vec![sample_select],
            4 => vec![0, 1],
            5 => vec![2, 3],
            _ => vec![0, 1, 2, 3],
        }
        .into_iter()
        .map(|s| s % available)
        .collect();
        let scale = 2f32.powi(exp_bias);

        // destination: 2D tiled, pitch aligned to 32, base adjusted by its macro tile phase (log2 bpp = 2)
        let dest_base = r(RB_COPY_DEST_BASE) & 0x1FFF_FFFF;
        let dest_pitch = (r(RB_COPY_DEST_PITCH) & 0x3FFF).next_multiple_of(32);
        let macro_tile_bytes_log2 = 2 * 5 + 2;
        let phase = (dest_base & 0xFFF) >> macro_tile_bytes_log2;
        let base = dest_base - (phase << macro_tile_bytes_log2);
        let x_phase = phase << 5;

        for y in y0 as u32..y1 as u32 {
            for x in x0 as u32..x1 as u32 {
                let mut sum = [0f32; 4];
                for &s in &selected {
                    let c = unpack_8888(self.samples[color.pixel_sample_index(x, y, s)]);
                    for k in 0..4 {
                        sum[k] += c[k];
                    }
                }
                let mut c = sum.map(|v| v / selected.len() as f32 * scale);
                if swap_rb {
                    c.swap(0, 2);
                }
                let offset = tiled_offset_2d(x + x_phase, y, dest_pitch, 2) as u64 * 4;
                let _ = bus.write_u32(base as u64 + offset, guest_endian_swap(pack_8888(c), endian));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surfaces_address_tiles_and_swap_depth_halves() {
        // xenos.h GetSurfacePitchTiles
        assert_eq!(surface_pitch_tiles(1280, 1, false), 16);
        assert_eq!(surface_pitch_tiles(640, 2, false), 16);
        assert_eq!(surface_pitch_tiles(1280, 0, true), 32);
        let color = Surface { base: 608, pitch_tiles: 16, msaa: 1, depth: false };
        assert_eq!(color.sample_index(0, 0), 608 * TILE_SAMPLES);
        assert_eq!(color.sample_index(81, 17), (608 + 16 + 1) * TILE_SAMPLES + 80 + 1);
        // the last tile row wraps to tile 0
        assert_eq!(color.sample_index(0, 90 * 16), (608 + 1440) % 2048 * TILE_SAMPLES);
        // 2x: pixel (3, 5) sample 1 is sample (3, 11)
        assert_eq!(color.pixel_sample_index(3, 5, 1), color.sample_index(3, 11));
        let depth = Surface { depth: true, ..color };
        assert_eq!(depth.sample_index(0, 0), 608 * TILE_SAMPLES + 40);
        assert_eq!(depth.sample_index(45, 1), 608 * TILE_SAMPLES + 80 + 5);
        // 4x: pixel (1, 1) sample 3 is sample (3, 3)
        let ms4 = Surface { msaa: 2, ..color };
        assert_eq!(ms4.pixel_sample_index(1, 1, 3), ms4.sample_index(3, 3));
    }

    #[test]
    fn row_bands_are_disjoint_unless_targets_alias() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, EDRAM_MODE_COLOR_DEPTH);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0001_0500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0x260);
        bus.gpu_regs.set(RB_COLOR_MASK, 0xF);
        // the dash's 1280x720 2x target ends exactly at the last tile
        assert!(state(&bus).pixels_are_disjoint(0, 719));
        assert!(!state(&bus).pixels_are_disjoint(0, 1100), "more rows than the EDRAM holds wrap onto themselves");
        // depth at tile 0: rows 0..299 (38 tile rows, 608 tiles) end where the color target starts; more rows reach it
        bus.gpu_regs.set(RB_DEPTHCONTROL, 2);
        bus.gpu_regs.set(RB_DEPTH_INFO, 0);
        assert!(state(&bus).pixels_are_disjoint(0, 299));
        assert!(!state(&bus).pixels_are_disjoint(0, 359));
        bus.gpu_regs.set(RB_DEPTH_INFO, 0x260 + 16 * 40);
        assert!(!state(&bus).pixels_are_disjoint(0, 359));
    }

    #[test]
    fn blend_control_fields_follow_registers_h() {
        // the dash's UI blend: color SrcAlpha / InvSrcAlpha add, alpha One / InvSrcAlpha add
        let b = BlendControl(0x0701_0706);
        let out = b.blend([1.0, 0.0, 0.0, 0.25], [0.0, 0.0, 1.0, 1.0], [0.0; 4]);
        assert_eq!(out, [0.25, 0.0, 0.75, 0.25 + 0.75]);
        // One / Zero: replace
        assert_eq!(BlendControl(0x0001_0001).blend([0.5; 4], [1.0; 4], [0.0; 4]), [0.5; 4]);
        // max ignores the factors
        assert_eq!(BlendControl(3 << 5).blend([0.2; 4], [0.6; 4], [0.0; 4])[0], 0.6);
    }

    fn state(bus: &MemoryBus) -> RenderState {
        RenderState::from_registers(bus).expect("bound")
    }

    #[test]
    fn depth_only_draw_writes_d24s8_with_stencil_replace() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, EDRAM_MODE_DEPTH_ONLY);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0002_0280);
        bus.gpu_regs.set(RB_DEPTH_INFO, 0x260);
        // the dash's clear: stencil and z enabled, z write, always/always, zpass replace
        bus.gpu_regs.set(RB_DEPTHCONTROL, 0x8777);
        bus.gpu_regs.set(RB_STENCILREFMASK, 0x00FF_0000 | 0x5A);
        let s = state(&bus);
        assert!(s.colors.is_empty());
        let mut edram = Edram::new();
        assert!(depth_stencil(edram.cells(), &s, 1, 1, 1.0, false));
        let (surface, _) = s.depth.unwrap();
        for k in 0..4 {
            assert_eq!(edram.samples[surface.pixel_sample_index(1, 1, k)], 0xFFFF_FF5A);
        }
        // LESS against the stored 1.0 fails for 1.0, passes for 0.5
        bus.gpu_regs.set(RB_DEPTHCONTROL, (1 << 4) | 6);
        let s = state(&bus);
        assert!(!depth_stencil(edram.cells(), &s, 1, 1, 1.0, false));
        assert!(depth_stencil(edram.cells(), &s, 1, 1, 0.5, false));
        assert_eq!(edram.samples[surface.pixel_sample_index(1, 1, 0)] >> 8, 0x80_0000);
    }

    #[test]
    fn color_writes_honor_mask_and_blend() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, EDRAM_MODE_COLOR_DEPTH);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0001_0500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0x260);
        bus.gpu_regs.set(RB_COLOR_MASK, 0x7); // no alpha
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0001_0001);
        let s = state(&bus);
        assert!(s.depth.is_none());
        let mut edram = Edram::new();
        let (_, surface, _) = s.colors[0];
        edram.samples[surface.pixel_sample_index(2, 3, 0)] = 0x7700_0000;
        write_colors(edram.cells(), &s, 2, 3, &[Some([1.0, 0.5, 0.0, 0.0]), None, None, None]);
        assert_eq!(edram.samples[surface.pixel_sample_index(2, 3, 0)], 0x7700_80FF);
        assert_eq!(edram.samples[surface.pixel_sample_index(2, 3, 1)], 0x0000_80FF);
    }

    #[test]
    fn resolve_follows_get_resolve_info() {
        let mut bus = MemoryBus::new();
        // the dash's resolve: 1280x720 2x surface at tile 608, k01, k_8_8_8_8 destination with R/B swap
        bus.gpu_regs.set(RB_MODECONTROL, EDRAM_MODE_COPY);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0001_0500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0x260);
        bus.gpu_regs.set(RB_COPY_CONTROL, 0x0010_0040 | 0x100);
        bus.gpu_regs.set(RB_COLOR_CLEAR, 0x1122_3344);
        bus.gpu_regs.set(RB_COPY_DEST_BASE, 0x0031_4000);
        bus.gpu_regs.set(RB_COPY_DEST_PITCH, 0x02D0_0500);
        bus.gpu_regs.set(RB_COPY_DEST_INFO, 0x0100_0300 | 2);
        bus.gpu_regs.set(PA_SC_WINDOW_SCISSOR_BR, 0x02D0_0500);
        bus.gpu_regs.set(PA_SC_SCREEN_SCISSOR_BR, 0x2000_2000);
        bus.gpu_regs.set(PA_SU_VTX_CNTL, 4);
        // vertex fetch 0 at 0x10000 (k8in32): (-0.5, -0.5), (15.5, -0.5), (15.5, 7.5) -> rectangle 0..16 x 0..8
        bus.gpu_regs.set(FETCH_00, 0x0001_0000 | 3);
        bus.gpu_regs.set(FETCH_00 + 1, (6 << 2) | 2);
        for (i, v) in [-0.5f32, -0.5, 15.5, -0.5, 15.5, 7.5].iter().enumerate() {
            bus.write_u32(0x10000 + 4 * i as u64, v.to_bits()).unwrap();
        }
        let mut edram = Edram::new();
        let color = Surface { base: 0x260, pitch_tiles: 16, msaa: 1, depth: false };
        // pixel (5, 2): samples red and blue -> average (0.5, 0, 0.5, 1)
        edram.samples[color.pixel_sample_index(5, 2, 0)] = pack_8888([1.0, 0.0, 0.0, 1.0]);
        edram.samples[color.pixel_sample_index(5, 2, 1)] = pack_8888([0.0, 0.0, 1.0, 1.0]);
        edram.samples[color.pixel_sample_index(1, 0, 0)] = pack_8888([1.0, 0.0, 0.0, 1.0]);
        edram.samples[color.pixel_sample_index(1, 0, 1)] = pack_8888([1.0, 0.0, 0.0, 1.0]);
        assert_eq!(edram.resolve(&mut bus), Some((0, 0, 16, 8)));
        let at = |x, y| bus.read_u32(0x0031_4000 + tiled_offset_2d(x, y, 1280, 2) as u64 * 4).unwrap();
        // k8in32: the guest reads the packed value; R/B swapped
        assert_eq!(at(1, 0), 0xFF00_00FF & 0xFF00_0000 | 0xFF << 16);
        assert_eq!(at(5, 2), 0xFF80_0080);
        // the rectangle was cleared afterwards
        assert_eq!(edram.samples[color.pixel_sample_index(5, 2, 1)], 0x1122_3344);
        assert_eq!(edram.samples[color.pixel_sample_index(16, 0, 0)], 0);
    }
}
