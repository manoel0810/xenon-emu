// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// Also referencing Mesa / Freedreno a2xx registers (MIT). See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/xenos.h              RB_DEPTHCONTROL, RB_BLENDCONTROL, RB_COLOR_MASK, RB_COPY_*
//   src/xenia/gpu/register_table.inc   register offsets
//   src/xenia/gpu/texture_util.cc      tiled resolve offsets
//
//! EDRAM buffer, Depth / Stencil testing, Alpha Blending, and Resolve backend for the Xenos GPU (Phase 4.5c).

use crate::gpu::texture::tiled_offset_2d;
use crate::gpu::xenos::guest_endian_swap;
use crate::memory::MemoryBus;

pub const EDRAM_WIDTH: usize = 1280;
pub const EDRAM_HEIGHT: usize = 720;

pub const RB_MODECONTROL: u32 = 0x2208;
pub const RB_SURFACE_INFO: u32 = 0x2000;
pub const RB_COLOR_INFO: u32 = 0x2001;
pub const RB_DEPTH_INFO: u32 = 0x2002;
pub const RB_COLOR_MASK: u32 = 0x2104;
pub const RB_DEPTHCONTROL: u32 = 0x2200;
pub const RB_BLENDCONTROL0: u32 = 0x2201;
pub const RB_COLORCONTROL: u32 = 0x2202;

pub const PA_SC_WINDOW_SCISSOR_TL: u32 = 0x2081;
pub const PA_SC_WINDOW_SCISSOR_BR: u32 = 0x2082;
pub const PA_SC_SCREEN_SCISSOR_TL: u32 = 0x200E;
pub const PA_SC_SCREEN_SCISSOR_BR: u32 = 0x200F;

pub const RB_COPY_CONTROL: u32 = 0x2318;
pub const RB_COPY_DEST_BASE: u32 = 0x2319;
pub const RB_COPY_DEST_PITCH: u32 = 0x231A;
pub const RB_COPY_DEST_INFO: u32 = 0x231B;
pub const RB_DEPTH_CLEAR: u32 = 0x231D;
pub const RB_COLOR_CLEAR: u32 = 0x231E;

/// EDRAM on-chip render buffer (Color target + Depth buffer).
#[derive(Debug, Clone)]
pub struct EdramBuffer {
    /// 1280x720 ARGB8 color buffer.
    pub color: Vec<u32>,
    /// 1280x720 32-bit float depth buffer.
    pub depth: Vec<f32>,
}

impl EdramBuffer {
    pub fn new() -> Self {
        Self {
            color: vec![0u32; EDRAM_WIDTH * EDRAM_HEIGHT],
            depth: vec![1.0f32; EDRAM_WIDTH * EDRAM_HEIGHT],
        }
    }

    pub fn clear_color(&mut self, val: u32) {
        self.color.fill(val);
    }

    pub fn clear_depth(&mut self, val: f32) {
        self.depth.fill(val);
    }
}

/// Decoded depth control settings (RB_DEPTHCONTROL).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepthControl {
    pub z_enable: bool,
    pub z_write_enable: bool,
    pub z_func: u32,
}

impl DepthControl {
    pub fn from_reg(val: u32) -> Self {
        Self {
            z_enable: (val & 1) != 0,
            z_write_enable: ((val >> 1) & 1) != 0,
            z_func: (val >> 4) & 7,
        }
    }

    /// Evaluates incoming fragment depth against the depth buffer value.
    #[inline(always)]
    pub fn test_depth(&self, incoming: f32, current: f32) -> bool {
        if !self.z_enable {
            return true;
        }
        match self.z_func {
            // 0: NEVER
            0 => false,
            // 1: LESS
            1 => incoming < current,
            // 2: EQUAL
            2 => (incoming - current).abs() < 1e-5,
            // 3: LEQUAL
            3 => incoming <= current + 1e-5,
            // 4: GREATER
            4 => incoming > current,
            // 5: NOTEQUAL
            5 => (incoming - current).abs() >= 1e-5,
            // 6: GEQUAL
            6 => incoming >= current - 1e-5,
            // 7: ALWAYS
            7 => true,
            _ => true,
        }
    }
}

/// Decoded alpha/color blend control settings (RB_BLENDCONTROL0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlendControl {
    pub color_blend_enable: bool,
    pub color_src_blend: u32,
    pub color_dest_blend: u32,
    pub color_comb_fcn: u32,
    pub alpha_blend_enable: bool,
    pub alpha_src_blend: u32,
    pub alpha_dest_blend: u32,
    pub alpha_comb_fcn: u32,
}

impl BlendControl {
    pub fn from_reg(val: u32) -> Self {
        Self {
            color_blend_enable: (val & 1) != 0,
            color_src_blend: (val >> 1) & 0xF,
            color_comb_fcn: (val >> 5) & 7,
            color_dest_blend: (val >> 8) & 0xF,
            alpha_blend_enable: ((val >> 12) & 1) != 0,
            alpha_src_blend: (val >> 13) & 0xF,
            alpha_comb_fcn: (val >> 17) & 7,
            alpha_dest_blend: (val >> 20) & 0xF,
        }
    }

    #[inline(always)]
    fn get_factor(factor_type: u32, src_c: f32, src_a: f32, dst_c: f32, dst_a: f32) -> f32 {
        match factor_type {
            // 0: ZERO
            0 => 0.0,
            // 1: ONE
            1 => 1.0,
            // 4: SRC_COLOR
            4 => src_c,
            // 5: ONE_MINUS_SRC_COLOR
            5 => 1.0 - src_c,
            // 6: SRC_ALPHA
            6 => src_a,
            // 7: ONE_MINUS_SRC_ALPHA
            7 => 1.0 - src_a,
            // 8: DST_COLOR
            8 => dst_c,
            // 9: ONE_MINUS_DST_COLOR
            9 => 1.0 - dst_c,
            // 10: DST_ALPHA
            10 => dst_a,
            // 11: ONE_MINUS_DST_ALPHA
            11 => 1.0 - dst_a,
            // 12: CONSTANT_COLOR
            12 => 1.0,
            // 13: ONE_MINUS_CONSTANT_COLOR
            13 => 0.0,
            // 14: CONSTANT_ALPHA
            14 => 1.0,
            // 15: ONE_MINUS_CONSTANT_ALPHA
            15 => 0.0,
            _ => 1.0,
        }
    }

    #[inline(always)]
    fn combine(fcn: u32, s: f32, fs: f32, d: f32, fd: f32) -> f32 {
        match fcn {
            // 0: ADD: src * Fs + dst * Fd
            0 => (s * fs + d * fd).clamp(0.0, 1.0),
            // 1: SUB: src * Fs - dst * Fd
            1 => (s * fs - d * fd).clamp(0.0, 1.0),
            // 2: MIN: min(src, dst)
            2 => s.min(d),
            // 3: MAX: max(src, dst)
            3 => s.max(d),
            // 4: REVSUB: dst * Fd - src * Fs
            4 => (d * fd - s * fs).clamp(0.0, 1.0),
            _ => (s * fs + d * fd).clamp(0.0, 1.0),
        }
    }

    /// Blends incoming fragment RGBA (values in [0.0, 1.0]) with destination pixel (0xAARRGGBB),
    /// honoring `color_mask` (bits 0: R, 1: G, 2: B, 3: A).
    #[inline(always)]
    pub fn blend_pixel(&self, src: [f32; 4], dst: u32, color_mask: u32) -> u32 {
        let dr = ((dst >> 16) & 0xFF) as f32 / 255.0;
        let dg = ((dst >> 8) & 0xFF) as f32 / 255.0;
        let db = (dst & 0xFF) as f32 / 255.0;
        let da = ((dst >> 24) & 0xFF) as f32 / 255.0;

        let (sr, sg, sb, sa) = (
            src[0].clamp(0.0, 1.0),
            src[1].clamp(0.0, 1.0),
            src[2].clamp(0.0, 1.0),
            src[3].clamp(0.0, 1.0),
        );

        let final_r = if self.color_blend_enable {
            let fs = Self::get_factor(self.color_src_blend, sr, sa, dr, da);
            let fd = Self::get_factor(self.color_dest_blend, sr, sa, dr, da);
            Self::combine(self.color_comb_fcn, sr, fs, dr, fd)
        } else {
            sr
        };

        let final_g = if self.color_blend_enable {
            let fs = Self::get_factor(self.color_src_blend, sg, sa, dg, da);
            let fd = Self::get_factor(self.color_dest_blend, sg, sa, dg, da);
            Self::combine(self.color_comb_fcn, sg, fs, dg, fd)
        } else {
            sg
        };

        let final_b = if self.color_blend_enable {
            let fs = Self::get_factor(self.color_src_blend, sb, sa, db, da);
            let fd = Self::get_factor(self.color_dest_blend, sb, sa, db, da);
            Self::combine(self.color_comb_fcn, sb, fs, db, fd)
        } else {
            sb
        };

        let final_a = if self.alpha_blend_enable {
            let fs = Self::get_factor(self.alpha_src_blend, sa, sa, da, da);
            let fd = Self::get_factor(self.alpha_dest_blend, sa, sa, da, da);
            Self::combine(self.alpha_comb_fcn, sa, fs, da, fd)
        } else {
            sa
        };

        let out_r = if (color_mask & 1) != 0 { (final_r * 255.0 + 0.5) as u32 } else { (dst >> 16) & 0xFF };
        let out_g = if (color_mask & 2) != 0 { (final_g * 255.0 + 0.5) as u32 } else { (dst >> 8) & 0xFF };
        let out_b = if (color_mask & 4) != 0 { (final_b * 255.0 + 0.5) as u32 } else { dst & 0xFF };
        let out_a = if (color_mask & 8) != 0 { (final_a * 255.0 + 0.5) as u32 } else { (dst >> 24) & 0xFF };

        (out_a << 24) | (out_r << 16) | (out_g << 8) | out_b
    }
}

/// Decodes the active scissor rectangle from registers, returning (min_x, min_y, max_x, max_y).
pub fn active_scissor(bus: &MemoryBus, width: usize, height: usize) -> (usize, usize, usize, usize) {
    let w_tl = bus.gpu_regs.get(PA_SC_WINDOW_SCISSOR_TL);
    let w_br = bus.gpu_regs.get(PA_SC_WINDOW_SCISSOR_BR);
    let s_tl = bus.gpu_regs.get(PA_SC_SCREEN_SCISSOR_TL);
    let s_br = bus.gpu_regs.get(PA_SC_SCREEN_SCISSOR_BR);

    let mut min_x = 0usize;
    let mut min_y = 0usize;
    let mut max_x = width.saturating_sub(1);
    let mut max_y = height.saturating_sub(1);

    if w_br != 0 {
        let w_min_x = (w_tl & 0x3FFF) as usize;
        let w_min_y = ((w_tl >> 16) & 0x3FFF) as usize;
        let w_max_x = (w_br & 0x3FFF) as usize;
        let w_max_y = ((w_br >> 16) & 0x3FFF) as usize;
        if w_max_x >= w_min_x && w_max_y >= w_min_y {
            min_x = min_x.max(w_min_x);
            min_y = min_y.max(w_min_y);
            max_x = max_x.min(w_max_x);
            max_y = max_y.min(w_max_y);
        }
    }

    if s_br != 0 {
        let s_min_x = (s_tl & 0x3FFF) as usize;
        let s_min_y = ((s_tl >> 16) & 0x3FFF) as usize;
        let s_max_x = (s_br & 0x3FFF) as usize;
        let s_max_y = ((s_br >> 16) & 0x3FFF) as usize;
        if s_max_x >= s_min_x && s_max_y >= s_min_y {
            min_x = min_x.max(s_min_x);
            min_y = min_y.max(s_min_y);
            max_x = max_x.min(s_max_x);
            max_y = max_y.min(s_max_y);
        }
    }

    (min_x, min_y, max_x, max_y)
}

/// Resolves EDRAM color buffer contents to guest memory at `RB_COPY_DEST_BASE`.
pub fn resolve_edram_to_guest(edram: &EdramBuffer, bus: &mut MemoryBus) {
    let dest_base = (bus.gpu_regs.get(RB_COPY_DEST_BASE) & 0x1FFF_FFFF) as u64;
    if dest_base == 0 {
        return;
    }
    let dest_info = bus.gpu_regs.get(RB_COPY_DEST_INFO);
    let endian = dest_info & 7;
    let linear = (dest_info >> 3) & 1 != 0;
    let swap_rb = (dest_info >> 8) & 1 != 0;

    let width_aligned = (EDRAM_WIDTH as u32 + 31) & !31;

    for y in 0..EDRAM_HEIGHT {
        for x in 0..EDRAM_WIDTH {
            let mut pixel = edram.color[y * EDRAM_WIDTH + x];
            if swap_rb {
                let a = pixel & 0xFF00_0000;
                let r = (pixel >> 16) & 0xFF;
                let g = (pixel >> 8) & 0xFF;
                let b = pixel & 0xFF;
                pixel = a | (b << 16) | (g << 8) | r;
            }
            let swapped = guest_endian_swap(pixel, endian);

            let offset = if linear {
                ((y * EDRAM_WIDTH + x) * 4) as u64
            } else {
                (tiled_offset_2d(x as u32, y as u32, width_aligned, 2) as u64) * 4
            };

            let _ = bus.write_u32(dest_base + offset, swapped);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_test_handles_less_and_lequal() {
        let dc_less = DepthControl { z_enable: true, z_write_enable: true, z_func: 1 };
        assert!(dc_less.test_depth(0.4, 0.5));
        assert!(!dc_less.test_depth(0.5, 0.5));
        assert!(!dc_less.test_depth(0.6, 0.5));

        let dc_lequal = DepthControl { z_enable: true, z_write_enable: true, z_func: 3 };
        assert!(dc_lequal.test_depth(0.5, 0.5));
        assert!(dc_lequal.test_depth(0.4, 0.5));
        assert!(!dc_lequal.test_depth(0.6, 0.5));
    }

    #[test]
    fn alpha_blending_srcalpha_invsrcalpha() {
        let blend = BlendControl {
            color_blend_enable: true,
            color_src_blend: 6, // SRC_ALPHA
            color_dest_blend: 7, // ONE_MINUS_SRC_ALPHA
            color_comb_fcn: 0, // ADD
            alpha_blend_enable: false,
            alpha_src_blend: 1,
            alpha_dest_blend: 0,
            alpha_comb_fcn: 0,
        };
        // Red with 50% alpha over solid Blue (0xFF0000FF)
        let src = [1.0, 0.0, 0.0, 0.5];
        let dst = 0xFF00_00FF;
        let result = blend.blend_pixel(src, dst, 0xF);

        let r = (result >> 16) & 0xFF;
        let b = result & 0xFF;
        // Expected: r ~ 128, b ~ 128
        assert!((r as i32 - 128).abs() <= 2, "r was {}", r);
        assert!((b as i32 - 128).abs() <= 2, "b was {}", b);
    }
}
