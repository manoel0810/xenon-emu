// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/registers.h  PA_SU_SC_MODE_CNTL (cull_front, cull_back, face, persp_corr_dis), RB_COLORCONTROL
//   src/xenia/gpu/xenos.h      CompareFunction (alpha test)
//
//! Software rasterizer for Xenos GPU triangles (Phase 4.5b), writing through the EDRAM render backend
//! (gpu/edram.rs). Pixel centers are sampled with the Direct3D top-left rule; depth is interpolated linearly in screen
//! space, interpolators perspective-correctly unless PA_SU_SC_MODE_CNTL.persp_corr_dis. Per pixel: pixel shader,
//! kill, alpha test, depth/stencil, then the color writes.
//! Also includes 2D overlay helpers used by the display's own drawing.

use crate::gpu::edram::{DepthResult, Edram, RenderState, EDRAM_MODE_COLOR_DEPTH, PA_SU_SC_MODE_CNTL};
use crate::gpu::primitive::ScreenVertex;
use crate::gpu::shader_env::GpuShaderEnv;
use crate::gpu::shader_interp::{run, ShaderState};
use crate::gpu::texture::GpuTextureSampler;
use crate::gpu::ucode::ShaderType;
use crate::memory::MemoryBus;

/// CompareFunction on floats (alpha test: `alpha FUNC reference`).
fn compare_f32(func: u32, a: f32, b: f32) -> bool {
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

/// Rasterizes one triangle into the bound render targets.
pub fn rasterize_triangle(
    edram: &mut Edram,
    state: &RenderState,
    tri: &[ScreenVertex; 3],
    bus: &MemoryBus,
    pixel_shader: &[u32],
    sampler: &mut GpuTextureSampler,
) {
    let area = (tri[1].x - tri[0].x) * (tri[2].y - tri[0].y) - (tri[1].y - tri[0].y) * (tri[2].x - tri[0].x);
    if area == 0.0 || !area.is_finite() {
        return;
    }
    // screen y grows downwards: a positive area is clockwise on screen; face 0 = counter-clockwise is front
    let su = bus.gpu_regs.get(PA_SU_SC_MODE_CNTL);
    let clockwise = area > 0.0;
    let front = if su & 4 == 0 { !clockwise } else { clockwise };
    if (front && su & 1 != 0) || (!front && su & 2 != 0) {
        return;
    }
    let perspective = su & (1 << 20) == 0;
    // order the vertices clockwise so that every edge function is positive inside
    let v = if clockwise { [&tri[0], &tri[1], &tri[2]] } else { [&tri[0], &tri[2], &tri[1]] };
    let area = area.abs();

    let (sx0, sy0, sx1, sy1) = state.scissor;
    let min_x = (v.iter().map(|p| p.x).fold(f32::INFINITY, f32::min).floor() as i32).max(sx0);
    let max_x = (v.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(sx1 - 1);
    let min_y = (v.iter().map(|p| p.y).fold(f32::INFINITY, f32::min).floor() as i32).max(sy0);
    let max_y = (v.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(sy1 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }

    // edge a->b: (bx - ax)(py - ay) - (by - ay)(px - ax), weighting the opposite vertex; a center exactly on a top
    // or left edge is inside
    let edges: [(usize, usize, usize); 3] = [(1, 2, 0), (2, 0, 1), (0, 1, 2)];
    let top_left = |a: &ScreenVertex, b: &ScreenVertex| {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        (dy == 0.0 && dx > 0.0) || dy < 0.0
    };
    let inputs: Vec<usize> = (0..16).filter(|&k| v.iter().any(|p| p.interpolators[k].is_some())).collect();
    let write_colors = state.mode == EDRAM_MODE_COLOR_DEPTH && !state.colors.is_empty();

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        for x in min_x..=max_x {
            let px = x as f32 + 0.5;
            let mut l = [0f32; 3];
            let mut inside = true;
            for &(a, b, opposite) in &edges {
                let e = (v[b].x - v[a].x) * (py - v[a].y) - (v[b].y - v[a].y) * (px - v[a].x);
                if e < 0.0 || (e == 0.0 && !top_left(v[a], v[b])) {
                    inside = false;
                    break;
                }
                l[opposite] = e / area;
            }
            if !inside {
                continue;
            }
            let mut z = l[0] * v[0].z + l[1] * v[1].z + l[2] * v[2].z;

            let mut ps = ShaderState::default();
            let weights = if perspective {
                let q = [l[0] * v[0].inv_w, l[1] * v[1].inv_w, l[2] * v[2].inv_w];
                let sum = q[0] + q[1] + q[2];
                if sum == 0.0 { l } else { q.map(|w| w / sum) }
            } else {
                l
            };
            for &k in &inputs {
                let value = |i: usize| v[i].interpolators[k].unwrap_or([0.0; 4]);
                let (a, b, c) = (value(0), value(1), value(2));
                ps.r[k] = std::array::from_fn(|i| weights[0] * a[i] + weights[1] * b[i] + weights[2] * c[i]);
            }
            let mut colors = [None; 4];
            if !pixel_shader.is_empty() {
                let mut env = GpuShaderEnv { bus, sampler: &mut *sampler };
                if run(pixel_shader, ShaderType::Pixel, &mut ps, &mut env).is_err() || ps.killed {
                    continue;
                }
                colors = ps.exports.colors;
                if let Some(depth) = ps.exports.depth {
                    z = depth;
                }
            }
            let (alpha_test, alpha_func) = state.alpha_test;
            if alpha_test && write_colors {
                if let Some(c) = colors[0] {
                    if !compare_f32(alpha_func, c[3], state.alpha_ref) {
                        continue;
                    }
                }
            }
            if let DepthResult::Fail = edram.depth_stencil(state, x as u32, y as u32, z, !front) {
                continue;
            }
            if write_colors {
                edram.write_colors(state, x as u32, y as u32, &colors);
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------
// 2D helpers used by the display's own overlay drawing (not by the GPU emulation).
// -------------------------------------------------------------------------------------------------

/// Fast alpha blending between source (with alpha in top 8 bits) and destination (RGB)
#[inline(always)]
pub fn blend_pixel(src: u32, dst: u32) -> u32 {
    let sa = (src >> 24) & 0xFF;
    if sa == 255 {
        return src & 0x00FF_FFFF;
    }
    if sa == 0 {
        return dst;
    }
    let inv_sa = 255 - sa;

    let sr = (src >> 16) & 0xFF;
    let sg = (src >> 8) & 0xFF;
    let sb = src & 0xFF;

    let dr = (dst >> 16) & 0xFF;
    let dg = (dst >> 8) & 0xFF;
    let db = dst & 0xFF;

    let r = (sr * sa + dr * inv_sa) / 255;
    let g = (sg * sa + dg * inv_sa) / 255;
    let b = (sb * sa + db * inv_sa) / 255;

    (r << 16) | (g << 8) | b
}

/// Rasterizes a rounded rectangle container (authentic Xbox 360 blade card style)
pub fn rasterize_rounded_card(
    fb: &mut [u32],
    fb_width: usize,
    fb_height: usize,
    rx: f32,
    ry: f32,
    rw: f32,
    rh: f32,
    corner_radius: f32,
    fill_top: u32,
    fill_bottom: u32,
    border_color: u32,
) {
    let min_x = rx.max(0.0) as usize;
    let max_x = ((rx + rw).min((fb_width - 1) as f32)) as usize;
    let min_y = ry.max(0.0) as usize;
    let max_y = ((ry + rh).min((fb_height - 1) as f32)) as usize;

    let cr = corner_radius;
    let (t_r, t_g, t_b) = (((fill_top >> 16) & 0xFF) as f32, ((fill_top >> 8) & 0xFF) as f32, (fill_top & 0xFF) as f32);
    let (b_r, b_g, b_b) = (((fill_bottom >> 16) & 0xFF) as f32, ((fill_bottom >> 8) & 0xFF) as f32, (fill_bottom & 0xFF) as f32);

    for y in min_y..=max_y {
        let ty = (y as f32 - ry) / rh;
        let row_offset = y * fb_width;

        let cur_r = (t_r + ty * (b_r - t_r)) as u32;
        let cur_g = (t_g + ty * (b_g - t_g)) as u32;
        let cur_b = (t_b + ty * (b_b - t_b)) as u32;
        let fill_color = 0xFF00_0000 | (cur_r << 16) | (cur_g << 8) | cur_b;

        for x in min_x..=max_x {
            let px = x as f32;
            let py = y as f32;

            // Check corner rounding
            let mut inside = true;
            let mut is_border = false;

            // Top-Left corner
            if px < rx + cr && py < ry + cr {
                let dx = px - (rx + cr);
                let dy = py - (ry + cr);
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > cr {
                    inside = false;
                } else if dist >= cr - 1.5 {
                    is_border = true;
                }
            }
            // Top-Right corner
            else if px > rx + rw - cr && py < ry + cr {
                let dx = px - (rx + rw - cr);
                let dy = py - (ry + cr);
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > cr {
                    inside = false;
                } else if dist >= cr - 1.5 {
                    is_border = true;
                }
            }
            // Bottom-Left corner
            else if px < rx + cr && py > ry + rh - cr {
                let dx = px - (rx + cr);
                let dy = py - (ry + rh - cr);
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > cr {
                    inside = false;
                } else if dist >= cr - 1.5 {
                    is_border = true;
                }
            }
            // Bottom-Right corner
            else if px > rx + rw - cr && py > ry + rh - cr {
                let dx = px - (rx + rw - cr);
                let dy = py - (ry + rh - cr);
                let dist = (dx * dx + dy * dy).sqrt();
                if dist > cr {
                    inside = false;
                } else if dist >= cr - 1.5 {
                    is_border = true;
                }
            }
            // Straight outer borders
            else if px == rx || px == rx + rw - 1.0 || py == ry || py == ry + rh - 1.0 {
                is_border = true;
            }

            if inside {
                let color = if is_border { border_color } else { fill_color };
                let dst = fb[row_offset + x];
                fb[row_offset + x] = blend_pixel(color, dst);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::edram::{unpack_8888, PA_SC_SCREEN_SCISSOR_BR, PA_SC_WINDOW_SCISSOR_BR, RB_BLENDCONTROLS, RB_COLOR_INFO, RB_COLOR_MASK, RB_MODECONTROL, RB_SURFACE_INFO};
    use crate::gpu::ucode::{SCALAR_RETAIN_PREV, VECTOR_MAX};
    use crate::gpu::ucode_test::{alu, cf, exec, pack, shader, Alu};

    fn vertex(x: f32, y: f32) -> ScreenVertex {
        let mut interpolators = [None; 16];
        interpolators[0] = Some([1.0, 0.0, 0.0, 1.0]);
        ScreenVertex { x, y, z: 0.5, inv_w: 1.0, interpolators }
    }

    #[test]
    fn rectangles_cover_exactly_their_pixels() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, 4);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0);
        bus.gpu_regs.set(RB_COLOR_MASK, 0xF);
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0001_0001);
        bus.gpu_regs.set(PA_SC_WINDOW_SCISSOR_BR, 0x02D0_0500);
        bus.gpu_regs.set(PA_SC_SCREEN_SCISSOR_BR, 0x2000_2000);
        let state = RenderState::from_registers(&bus).unwrap();
        let mut edram = Edram::new();
        let mut sampler = GpuTextureSampler::new(&bus);
        // pixel shader: oC0 = r0 (the first interpolator)
        let export = alu(Alu { export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [0, 0, 0], temp: [true; 3], ..Default::default() });
        let ps = shader(&[pack(exec(2, 1, 1, 0, 1 << 9), cf(0, 0, 0)), export]);
        // a 4x2 rectangle at (2, 2) as two triangles sharing a diagonal, with opposite windings
        let (a, b, c, d) = (vertex(2.0, 2.0), vertex(6.0, 2.0), vertex(6.0, 4.0), vertex(2.0, 4.0));
        rasterize_triangle(&mut edram, &state, &[a.clone(), b, c.clone()], &bus, &ps, &mut sampler);
        rasterize_triangle(&mut edram, &state, &[a, d, c], &bus, &ps, &mut sampler);
        let (_, surface, _) = state.colors[0];
        let covered: Vec<(u32, u32)> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|&(x, y)| edram.samples[surface.pixel_sample_index(x, y, 0)] != 0)
            .collect();
        let expected: Vec<(u32, u32)> = (2..4).flat_map(|y| (2..6).map(move |x| (x, y))).collect();
        assert_eq!(covered, expected);
        assert_eq!(unpack_8888(edram.samples[surface.pixel_sample_index(3, 3, 0)]), [1.0, 0.0, 0.0, 1.0]);
        // a pixel on the shared diagonal was written once: blending dst + src would show otherwise
        drop(sampler);
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0101_0101);
        let state = RenderState::from_registers(&bus).unwrap();
        let mut sampler = GpuTextureSampler::new(&bus);
        let mut edram = Edram::new();
        let (a, b, c, d) = (vertex(0.0, 0.0), vertex(4.0, 0.0), vertex(4.0, 4.0), vertex(0.0, 4.0));
        let half = |mut v: ScreenVertex| {
            v.interpolators[0] = Some([0.5, 0.0, 0.0, 0.5]);
            v
        };
        rasterize_triangle(&mut edram, &state, &[half(a.clone()), half(b), half(c.clone())], &bus, &ps, &mut sampler);
        rasterize_triangle(&mut edram, &state, &[half(a), half(d), half(c)], &bus, &ps, &mut sampler);
        assert!((0..4).all(|i| edram.samples[surface.pixel_sample_index(i, i, 0)] == 0x8000_0080));
    }
}
