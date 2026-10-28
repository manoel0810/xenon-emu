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

use crate::gpu::edram::{depth_stencil, write_colors, Edram, RenderState, EDRAM_MODE_COLOR_DEPTH, PA_SU_SC_MODE_CNTL};
use crate::gpu::primitive::ScreenVertex;
use crate::gpu::shader_env::GpuShaderEnv;
use crate::gpu::shader_interp::{BatchShader, BatchState, CompiledShader, ShaderState, Vec4, LANES};
use crate::gpu::texture::GpuTextureSampler;
use crate::memory::MemoryBus;
use std::sync::atomic::AtomicU32;

/// Below this many pixels in its bounding box a triangle is rasterized on the calling thread.
const PARALLEL_MIN_PIXELS: i64 = 16 * 1024;

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

/// A triangle set up for scanning: clockwise vertices, area, bounds and per-draw state.
struct Setup<'a> {
    v: [&'a ScreenVertex; 3],
    area: f32,
    front: bool,
    perspective: bool,
    /// Interpolators any vertex exports: (register, value at each vertex, 0 where a vertex does not export it).
    inputs: Vec<(usize, [Vec4; 3])>,
    write_colors: bool,
}

impl Setup<'_> {
    /// Scans rows y0, y0 + step, ... up to y1 (columns x0..=x1) of the triangle.
    #[allow(clippy::too_many_arguments)]
    fn scan(&self, samples: &[AtomicU32], state: &RenderState, bus: &MemoryBus, pixel_shader: Option<&CompiledShader>, batch: Option<&BatchShader>, (x0, x1): (i32, i32), y0: i32, y1: i32, step: usize) {
        if let (Some(batch), Some(_)) = (batch, pixel_shader) {
            return self.scan_batched(samples, state, bus, batch, (x0, x1), y0, y1, step);
        }
        let v = &self.v;
        let mut sampler = GpuTextureSampler::new(bus);
        // edge a->b: (bx - ax)(py - ay) - (by - ay)(px - ax), weighting the opposite vertex; a center exactly on a
        // top or left edge is inside
        let edges: [(usize, usize, usize); 3] = [(1, 2, 0), (2, 0, 1), (0, 1, 2)];
        let top_left = |a: &ScreenVertex, b: &ScreenVertex| {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            (dy == 0.0 && dx > 0.0) || dy < 0.0
        };
        let mut ps = ShaderState::default();
        let temps = pixel_shader.map_or(64, |p| p.temps());
        for y in (y0..=y1).step_by(step) {
            let py = y as f32 + 0.5;
            // a triangle is convex: once a row has left it, the rest of the row is outside
            let mut entered = false;
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let mut l = [0f32; 3];
                let mut inside = true;
                for &(a, b, opposite) in &edges {
                    let e = (v[b].x - v[a].x) * (py - v[a].y) - (v[b].y - v[a].y) * (px - v[a].x);
                    if e < 0.0 || (e == 0.0 && !top_left(v[a], v[b])) {
                        inside = false;
                        break;
                    }
                    l[opposite] = e / self.area;
                }
                if !inside {
                    if entered {
                        break;
                    }
                    continue;
                }
                entered = true;
                let mut z = l[0] * v[0].z + l[1] * v[1].z + l[2] * v[2].z;

                ps.reset_pixel(temps);
                let weights = if self.perspective {
                    let q = [l[0] * v[0].inv_w, l[1] * v[1].inv_w, l[2] * v[2].inv_w];
                    let sum = q[0] + q[1] + q[2];
                    if sum == 0.0 { l } else { q.map(|w| w / sum) }
                } else {
                    l
                };
                for &(k, [a, b, c]) in &self.inputs {
                    ps.r[k] = std::array::from_fn(|i| weights[0] * a[i] + weights[1] * b[i] + weights[2] * c[i]);
                }
                let mut colors = [None; 4];
                if let Some(pixel_shader) = pixel_shader {
                    let mut env = GpuShaderEnv { bus, sampler: &mut sampler };
                    if pixel_shader.run(&mut ps, &mut env).is_err() || ps.killed {
                        continue;
                    }
                    colors = ps.exports.colors;
                    if let Some(depth) = ps.exports.depth {
                        z = depth;
                    }
                }
                self.finish(samples, state, x, y, z, &colors);
            }
        }
    }

    /// The rest of a shaded pixel: alpha test, depth/stencil, then the color writes.
    #[inline(always)]
    fn finish(&self, samples: &[AtomicU32], state: &RenderState, x: i32, y: i32, z: f32, colors: &[Option<Vec4>; 4]) {
        let (alpha_test, alpha_func) = state.alpha_test;
        if alpha_test && self.write_colors {
            if let Some(c) = colors[0] {
                if !compare_f32(alpha_func, c[3], state.alpha_ref) {
                    return;
                }
            }
        }
        if !depth_stencil(samples, state, x as u32, y as u32, z, !self.front) {
            return;
        }
        if self.write_colors {
            write_colors(samples, state, x as u32, y as u32, colors);
        }
    }

    /// `scan` with a batched pixel shader: the covered pixels of a row are shaded `LANES` at a time, then finished
    /// in order, which gives every pixel the same result as `scan`.
    #[allow(clippy::too_many_arguments)]
    fn scan_batched(&self, samples: &[AtomicU32], state: &RenderState, bus: &MemoryBus, batch: &BatchShader, (x0, x1): (i32, i32), y0: i32, y1: i32, step: usize) {
        let v = &self.v;
        let mut sampler = GpuTextureSampler::new(bus);
        let edges: [(usize, usize, usize); 3] = [(1, 2, 0), (2, 0, 1), (0, 1, 2)];
        let top_left = |a: &ScreenVertex, b: &ScreenVertex| {
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            (dy == 0.0 && dx > 0.0) || dy < 0.0
        };
        let mut st = BatchState::default();
        let temps = batch.temps();
        let mut xs = [0i32; LANES];
        let mut zs = [0f32; LANES];
        let mut lanes = 0usize;
        let mut flush = |st: &mut BatchState, lanes: &mut usize, xs: &[i32; LANES], zs: &[f32; LANES], y: i32| {
            if *lanes == 0 {
                return;
            }
            let mut env = GpuShaderEnv { bus, sampler: &mut sampler };
            batch.run(st, *lanes, &mut env);
            for lane in 0..*lanes {
                if st.killed[lane] {
                    continue;
                }
                let z = st.depth[lane].unwrap_or(zs[lane]);
                self.finish(samples, state, xs[lane], y, z, &st.colors[lane]);
            }
            *lanes = 0;
        };
        for y in (y0..=y1).step_by(step) {
            let py = y as f32 + 0.5;
            let mut entered = false;
            for x in x0..=x1 {
                let px = x as f32 + 0.5;
                let mut l = [0f32; 3];
                let mut inside = true;
                for &(a, b, opposite) in &edges {
                    let e = (v[b].x - v[a].x) * (py - v[a].y) - (v[b].y - v[a].y) * (px - v[a].x);
                    if e < 0.0 || (e == 0.0 && !top_left(v[a], v[b])) {
                        inside = false;
                        break;
                    }
                    l[opposite] = e / self.area;
                }
                if !inside {
                    if entered {
                        break;
                    }
                    continue;
                }
                entered = true;
                if lanes == 0 {
                    st.reset(temps, LANES);
                }
                zs[lanes] = l[0] * v[0].z + l[1] * v[1].z + l[2] * v[2].z;
                xs[lanes] = x;
                let weights = if self.perspective {
                    let q = [l[0] * v[0].inv_w, l[1] * v[1].inv_w, l[2] * v[2].inv_w];
                    let sum = q[0] + q[1] + q[2];
                    if sum == 0.0 { l } else { q.map(|w| w / sum) }
                } else {
                    l
                };
                for &(k, [a, b, c]) in &self.inputs {
                    st.r[k][lanes] = std::array::from_fn(|i| weights[0] * a[i] + weights[1] * b[i] + weights[2] * c[i]);
                }
                lanes += 1;
                if lanes == LANES {
                    flush(&mut st, &mut lanes, &xs, &zs, y);
                }
            }
            flush(&mut st, &mut lanes, &xs, &zs, y);
        }
    }
}

/// Rasterizes one triangle into the bound render targets. Large triangles are scanned by up to `threads` threads,
/// each taking every n-th row (interleaved, so the threads share a triangle's wide and narrow rows evenly), when no
/// two of their pixels can share an EDRAM sample; every pixel is still processed once, so the result does not depend
/// on the thread count.
/// `pixel_shader` is compiled once per draw (None: no pixel shader loaded, nothing exported).
#[allow(clippy::too_many_arguments)]
pub fn rasterize_triangle(edram: &mut Edram, state: &RenderState, tri: &[ScreenVertex; 3], bus: &MemoryBus, pixel_shader: Option<&CompiledShader>, batch: Option<&BatchShader>, threads: usize) {
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
    // order the vertices clockwise so that every edge function is positive inside
    let v = if clockwise { [&tri[0], &tri[1], &tri[2]] } else { [&tri[0], &tri[2], &tri[1]] };
    let (sx0, sy0, sx1, sy1) = state.scissor;
    let min_x = (v.iter().map(|p| p.x).fold(f32::INFINITY, f32::min).floor() as i32).max(sx0);
    let max_x = (v.iter().map(|p| p.x).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(sx1 - 1);
    let min_y = (v.iter().map(|p| p.y).fold(f32::INFINITY, f32::min).floor() as i32).max(sy0);
    let max_y = (v.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max).ceil() as i32).min(sy1 - 1);
    if min_x > max_x || min_y > max_y {
        return;
    }
    let setup = Setup {
        v,
        area: area.abs(),
        front,
        perspective: su & (1 << 20) == 0,
        inputs: (0..16)
            .filter(|&k| v.iter().any(|p| p.interpolators[k].is_some()))
            .map(|k| (k, [0, 1, 2].map(|i| v[i].interpolators[k].unwrap_or([0.0; 4]))))
            .collect(),
        write_colors: state.mode == EDRAM_MODE_COLOR_DEPTH && !state.colors.is_empty(),
    };
    let samples = edram.cells();
    let rows = (max_y - min_y + 1) as i64;
    let pixels = rows * (max_x - min_x + 1) as i64;
    let bands = threads.min(rows as usize).max(1);
    if bands == 1 || pixels < PARALLEL_MIN_PIXELS || !state.pixels_are_disjoint(min_y as u32, max_y as u32) {
        setup.scan(samples, state, bus, pixel_shader, batch, (min_x, max_x), min_y, max_y, 1);
        return;
    }
    std::thread::scope(|scope| {
        for band in 0..bands as i32 {
            let setup = &setup;
            scope.spawn(move || setup.scan(samples, state, bus, pixel_shader, batch, (min_x, max_x), min_y + band, max_y, bands));
        }
    });
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
    use crate::gpu::ucode::{ShaderType, SCALAR_RETAIN_PREV, VECTOR_MAX};
    use crate::gpu::ucode_test::{alu, cf, exec, pack, shader, Alu};

    fn compile_ps(bus: &MemoryBus, words: &[u32]) -> CompiledShader {
        let mut sampler = GpuTextureSampler::new(bus);
        CompiledShader::compile(words, ShaderType::Pixel, &GpuShaderEnv { bus, sampler: &mut sampler })
    }

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
        // pixel shader: oC0 = r0 (the first interpolator)
        let export = alu(Alu { export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [0, 0, 0], temp: [true; 3], ..Default::default() });
        let ps = shader(&[pack(exec(2, 1, 1, 0, 1 << 9), cf(0, 0, 0)), export]);
        let ps = compile_ps(&bus, &ps);
        let ps = Some(&ps);
        // a 4x2 rectangle at (2, 2) as two triangles sharing a diagonal, with opposite windings
        let (a, b, c, d) = (vertex(2.0, 2.0), vertex(6.0, 2.0), vertex(6.0, 4.0), vertex(2.0, 4.0));
        rasterize_triangle(&mut edram, &state, &[a.clone(), b, c.clone()], &bus, ps, None, 4);
        rasterize_triangle(&mut edram, &state, &[a, d, c], &bus, ps, None, 4);
        let (_, surface, _) = state.colors[0];
        let covered: Vec<(u32, u32)> = (0..8)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|&(x, y)| edram.samples[surface.pixel_sample_index(x, y, 0)] != 0)
            .collect();
        let expected: Vec<(u32, u32)> = (2..4).flat_map(|y| (2..6).map(move |x| (x, y))).collect();
        assert_eq!(covered, expected);
        assert_eq!(unpack_8888(edram.samples[surface.pixel_sample_index(3, 3, 0)]), [1.0, 0.0, 0.0, 1.0]);
        // a pixel on the shared diagonal was written once: blending dst + src would show otherwise
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0101_0101);
        let state = RenderState::from_registers(&bus).unwrap();
        let mut edram = Edram::new();
        let (a, b, c, d) = (vertex(0.0, 0.0), vertex(4.0, 0.0), vertex(4.0, 4.0), vertex(0.0, 4.0));
        let half = |mut v: ScreenVertex| {
            v.interpolators[0] = Some([0.5, 0.0, 0.0, 0.5]);
            v
        };
        rasterize_triangle(&mut edram, &state, &[half(a.clone()), half(b), half(c.clone())], &bus, ps, None, 4);
        rasterize_triangle(&mut edram, &state, &[half(a), half(d), half(c)], &bus, ps, None, 4);
        assert!((0..4).all(|i| edram.samples[surface.pixel_sample_index(i, i, 0)] == 0x8000_0080));
    }

    #[test]
    fn threads_produce_the_serial_result() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, 4);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0001_0500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0x260);
        bus.gpu_regs.set(RB_COLOR_MASK, 0xF);
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0701_0706);
        bus.gpu_regs.set(PA_SC_WINDOW_SCISSOR_BR, 0x02D0_0500);
        bus.gpu_regs.set(PA_SC_SCREEN_SCISSOR_BR, 0x2000_2000);
        let state = RenderState::from_registers(&bus).unwrap();
        let export = alu(Alu { export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [0, 0, 0], temp: [true; 3], ..Default::default() });
        let ps = shader(&[pack(exec(2, 1, 1, 0, 1 << 9), cf(0, 0, 0)), export]);
        let ps = compile_ps(&bus, &ps);
        let ps = Some(&ps);
        let corner = |x: f32, y: f32, c: [f32; 4]| {
            let mut v = vertex(x, y);
            v.interpolators[0] = Some(c);
            v
        };
        let tri = [corner(-10.5, 3.0, [1.0, 0.0, 0.0, 0.5]), corner(1290.0, 100.0, [0.0, 1.0, 0.0, 0.25]), corner(200.0, 719.7, [0.0, 0.0, 1.0, 1.0])];
        let run = |threads| {
            let mut edram = Edram::new();
            rasterize_triangle(&mut edram, &state, &tri, &bus, ps, None, threads);
            rasterize_triangle(&mut edram, &state, &[tri[2].clone(), tri[0].clone(), corner(1279.0, 719.0, [0.5; 4])], &bus, ps, None, threads);
            edram.samples
        };
        assert!(run(1) == run(8));
    }

    #[test]
    fn batched_shading_produces_the_per_pixel_result() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, 4);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0001_0500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0x260);
        bus.gpu_regs.set(RB_COLOR_MASK, 0xF);
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0701_0706);
        bus.gpu_regs.set(PA_SC_WINDOW_SCISSOR_BR, 0x02D0_0500);
        bus.gpu_regs.set(PA_SC_SCREEN_SCISSOR_BR, 0x2000_2000);
        let state = RenderState::from_registers(&bus).unwrap();
        // r1 = r0 * r0; oC0 = max(r1, r0)
        let square = alu(Alu { vdst: 1, vmask: 0xF, vop: 1, sop: SCALAR_RETAIN_PREV, reg: [0, 0, 0], temp: [true; 3], ..Default::default() });
        let export = alu(Alu { export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [1, 0, 0], temp: [true; 3], ..Default::default() });
        let ps = compile_ps(&bus, &shader(&[pack(exec(2, 1, 2, 0, 1 << 9), cf(0, 0, 0)), square, export]));
        let mut sampler = GpuTextureSampler::new(&bus);
        let batch = ps.batch(&GpuShaderEnv { bus: &bus, sampler: &mut sampler }).expect("batchable");
        let corner = |x: f32, y: f32, c: [f32; 4]| {
            let mut v = vertex(x, y);
            v.interpolators[0] = Some(c);
            v
        };
        let tri = [corner(-10.5, 3.0, [1.0, 0.0, 0.0, 0.5]), corner(1290.0, 100.0, [0.0, 1.0, 0.0, 0.25]), corner(200.0, 719.7, [0.0, 0.0, 1.0, 1.0])];
        let run = |batch: Option<&BatchShader>, threads| {
            let mut edram = Edram::new();
            rasterize_triangle(&mut edram, &state, &tri, &bus, Some(&ps), batch, threads);
            edram.samples
        };
        let per_pixel = run(None, 1);
        assert!(per_pixel == run(Some(&batch), 1));
        assert!(per_pixel == run(Some(&batch), 8));
    }

    /// Full-screen cost: `XENON_SHADER_DIR=<dir> cargo test --release --lib bench_full_screen -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_full_screen() {
        let dir = std::env::var("XENON_SHADER_DIR").expect("XENON_SHADER_DIR");
        let words: Vec<u32> = std::fs::read(format!("{}/ps_6f16dffc30c02272.bin", dir)).unwrap().chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_MODECONTROL, 4);
        bus.gpu_regs.set(RB_SURFACE_INFO, 0x0001_0500);
        bus.gpu_regs.set(RB_COLOR_INFO, 0x260);
        bus.gpu_regs.set(RB_COLOR_MASK, 0xF);
        bus.gpu_regs.set(RB_BLENDCONTROLS[0], 0x0701_0706);
        bus.gpu_regs.set(PA_SC_WINDOW_SCISSOR_BR, 0x02D0_0500);
        bus.gpu_regs.set(PA_SC_SCREEN_SCISSOR_BR, 0x2000_2000);
        for i in 0..512u32 {
            bus.gpu_regs.set(0x4000 + i, (0.5 + (i % 7) as f32 * 0.1).to_bits());
        }
        let state = RenderState::from_registers(&bus).unwrap();
        let corner = |x: f32, y: f32| {
            let mut v = vertex(x, y);
            v.interpolators[0] = Some([x / 1280.0, y / 720.0, 0.0, 0.0]);
            v
        };
        let quad = [corner(0.0, 0.0), corner(1280.0, 0.0), corner(1280.0, 720.0), corner(0.0, 720.0)];
        let words = compile_ps(&bus, &words);
        let words = Some(&words);
        for threads in [1usize, 12] {
            let mut edram = Edram::new();
            let start = std::time::Instant::now();
            rasterize_triangle(&mut edram, &state, &[quad[0].clone(), quad[1].clone(), quad[2].clone()], &bus, words, None, threads);
            rasterize_triangle(&mut edram, &state, &[quad[0].clone(), quad[2].clone(), quad[3].clone()], &bus, words, None, threads);
            println!("{} thread(s): {:.1} ms per 1280x720 layer ({:.0} ns/pixel)", threads, start.elapsed().as_secs_f64() * 1000.0, start.elapsed().as_nanos() as f64 / (1280.0 * 720.0));
        }
    }
}
