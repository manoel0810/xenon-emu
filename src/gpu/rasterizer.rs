// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de.
//
//! Software rasterizer for Xenos GPU 3D primitives (Phase 4.5b).
//! Also includes 2D overlay helpers used by the display's own drawing.

use crate::gpu::edram::{active_scissor, BlendControl, DepthControl, EdramBuffer, EDRAM_HEIGHT, EDRAM_WIDTH, RB_BLENDCONTROL0, RB_COLOR_MASK, RB_DEPTHCONTROL};
use crate::gpu::primitive::Triangle;
use crate::gpu::shader_env::GpuShaderEnv;
use crate::gpu::shader_interp::{run, ShaderState};
use crate::gpu::texture::GpuTextureSampler;
use crate::gpu::ucode::ShaderType;
use crate::memory::MemoryBus;

const PA_SU_SC_MODE_CNTL: u32 = 0x2205;

/// Rasterizes a single 3D triangle into the EDRAM buffer with perspective-correct interpolation,
/// depth testing, pixel shader execution and alpha blending.
pub fn rasterize_triangle(
    edram: &mut EdramBuffer,
    tri: &Triangle,
    bus: &MemoryBus,
    pixel_shader: &[u32],
    sampler: &mut GpuTextureSampler,
) {
    let (v0, v1, v2) = (&tri.v[0], &tri.v[1], &tri.v[2]);
    let (x0, y0) = v0.screen;
    let (x1, y1) = v1.screen;
    let (x2, y2) = v2.screen;

    // Signed triangle area / determinant
    let area = (x1 - x0) * (y2 - y0) - (y1 - y0) * (x2 - x0);
    if area.abs() < 1e-5 {
        return; // Degenerate triangle
    }

    // Backface / Frontface culling check against PA_SU_SC_MODE_CNTL
    let su_cntl = bus.gpu_regs.get(PA_SU_SC_MODE_CNTL);
    let cull_front = (su_cntl & 1) != 0;
    let cull_back = (su_cntl & 2) != 0;
    let face_cw = (su_cntl & 4) != 0; // 0 = CCW is front, 1 = CW is front

    let is_ccw = area > 0.0;
    let is_front = if face_cw { !is_ccw } else { is_ccw };

    if (cull_front && is_front) || (cull_back && !is_front) {
        return;
    }

    // Active scissor clipping
    let (sc_min_x, sc_min_y, sc_max_x, sc_max_y) = active_scissor(bus, EDRAM_WIDTH, EDRAM_HEIGHT);

    // Screen-space bounding box
    let min_x = (x0.min(x1).min(x2).floor() as isize).max(sc_min_x as isize).max(0) as usize;
    let max_x = (x0.max(x1).max(x2).ceil() as isize).min(sc_max_x as isize).min((EDRAM_WIDTH - 1) as isize).max(0) as usize;
    let min_y = (y0.min(y1).min(y2).floor() as isize).max(sc_min_y as isize).max(0) as usize;
    let max_y = (y0.max(y1).max(y2).ceil() as isize).min(sc_max_y as isize).min((EDRAM_HEIGHT - 1) as isize).max(0) as usize;

    if min_x > max_x || min_y > max_y {
        return;
    }

    let depth_ctl = DepthControl::from_reg(bus.gpu_regs.get(RB_DEPTHCONTROL));
    let blend_ctl = BlendControl::from_reg(bus.gpu_regs.get(RB_BLENDCONTROL0));
    let raw_mask = bus.gpu_regs.get(RB_COLOR_MASK);
    let color_mask = if raw_mask == 0 { 0x0F } else { raw_mask & 0x0F };

    // Edge function deltas
    let dx01 = x1 - x0;
    let dy01 = y1 - y0;
    let dx12 = x2 - x1;
    let dy12 = y2 - y1;
    let dx20 = x0 - x2;
    let dy20 = y0 - y2;

    let inv_area = 1.0 / area;

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row_offset = y * EDRAM_WIDTH;

        for x in min_x..=max_x {
            let px = x as f32 + 0.5;

            // Edge functions matching barycentric definition: E_AB(P) = dx_AB * (py - yA) - dy_AB * (px - xA)
            let e12 = dx12 * (py - y1) - dy12 * (px - x1);
            let e20 = dx20 * (py - y2) - dy20 * (px - x2);
            let e01 = dx01 * (py - y0) - dy01 * (px - x0);

            let inside = if area > 0.0 {
                e12 >= 0.0 && e20 >= 0.0 && e01 >= 0.0
            } else {
                e12 <= 0.0 && e20 <= 0.0 && e01 <= 0.0
            };

            if !inside {
                continue;
            }

            // Barycentric coordinates
            let l0 = e12 * inv_area;
            let l1 = e20 * inv_area;
            let l2 = e01 * inv_area;

            // Perspective-correct interpolation weight
            let inv_w = l0 * v0.inv_w + l1 * v1.inv_w + l2 * v2.inv_w;
            if inv_w.abs() < 1e-7 {
                continue;
            }
            let w = 1.0 / inv_w;

            // Interpolated screen depth
            let z = (l0 * v0.z * v0.inv_w + l1 * v1.z * v1.inv_w + l2 * v2.z * v2.inv_w) * w;

            let pixel_idx = row_offset + x;
            let current_depth = edram.depth[pixel_idx];

            // Depth test
            if !depth_ctl.test_depth(z, current_depth) {
                continue;
            }

            // Interpolate vertex shader export interpolators (o0..o15)
            let mut ps_state = ShaderState::default();
            for k in 0..16 {
                let (has0, has1, has2) = (
                    v0.interpolators[k].is_some(),
                    v1.interpolators[k].is_some(),
                    v2.interpolators[k].is_some(),
                );
                if has0 || has1 || has2 {
                    let i0 = v0.interpolators[k].unwrap_or([0.0; 4]);
                    let i1 = v1.interpolators[k].unwrap_or([0.0; 4]);
                    let i2 = v2.interpolators[k].unwrap_or([0.0; 4]);
                    let mut interp = [0.0f32; 4];
                    for c in 0..4 {
                        interp[c] = (l0 * i0[c] * v0.inv_w + l1 * i1[c] * v1.inv_w + l2 * i2[c] * v2.inv_w) * w;
                    }
                    ps_state.r[k] = interp;
                }
            }

            // Execute active Pixel Shader
            let color = if pixel_shader.is_empty() {
                // Default to first interpolator or solid white
                let r0 = ps_state.r[0];
                if r0 != [0.0; 4] {
                    r0
                } else {
                    [1.0, 1.0, 1.0, 1.0]
                }
            } else {
                let mut env = GpuShaderEnv { bus, sampler };
                if run(pixel_shader, ShaderType::Pixel, &mut ps_state, &mut env).is_err() {
                    continue;
                }
                if ps_state.killed {
                    continue;
                }
                ps_state.exports.colors[0].unwrap_or([1.0, 1.0, 1.0, 1.0])
            };

            // Depth write
            if depth_ctl.z_write_enable {
                edram.depth[pixel_idx] = z;
            }

            // Alpha blending & Color write
            let dst_pixel = edram.color[pixel_idx];
            edram.color[pixel_idx] = blend_ctl.blend_pixel(color, dst_pixel, color_mask);
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
    use crate::gpu::primitive::Vertex;

    #[test]
    fn rasterizes_simple_triangle_with_depth_and_color() {
        let mut edram = EdramBuffer::new();
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(RB_DEPTHCONTROL, 0x13); // z_enable = true, z_write_enable = true, z_func = LESS (1)
        let mut sampler = GpuTextureSampler::new(&bus);

        let v0 = Vertex {
            screen: (10.0, 10.0),
            z: 0.2,
            inv_w: 1.0,
            interpolators: {
                let mut a = [None; 16];
                a[0] = Some([1.0, 0.0, 0.0, 1.0]); // Red
                a
            },
            ..Default::default()
        };
        let v1 = Vertex {
            screen: (30.0, 10.0),
            z: 0.2,
            inv_w: 1.0,
            interpolators: {
                let mut a = [None; 16];
                a[0] = Some([1.0, 0.0, 0.0, 1.0]);
                a
            },
            ..Default::default()
        };
        let v2 = Vertex {
            screen: (10.0, 30.0),
            z: 0.2,
            inv_w: 1.0,
            interpolators: {
                let mut a = [None; 16];
                a[0] = Some([1.0, 0.0, 0.0, 1.0]);
                a
            },
            ..Default::default()
        };

        let tri = Triangle { v: [v0, v1, v2] };
        rasterize_triangle(&mut edram, &tri, &bus, &[], &mut sampler);

        // Pixel (12, 12) should be covered and red (0xFFFF0000)
        let pixel = edram.color[12 * EDRAM_WIDTH + 12];
        assert_eq!(pixel, 0xFFFF_0000, "pixel at (12, 12) was 0x{:08X}", pixel);
        assert!((edram.depth[12 * EDRAM_WIDTH + 12] - 0.2).abs() < 1e-4);

        // Pixel (50, 50) outside triangle should remain 0
        assert_eq!(edram.color[50 * EDRAM_WIDTH + 50], 0);
    }
}
