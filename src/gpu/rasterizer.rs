use crate::gpu::xenos::Vertex;

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

/// Signed 2D edge function for barycentric triangle rasterization
#[inline(always)]
fn edge_func(ax: f32, ay: f32, bx: f32, by: f32, cx: f32, cy: f32) -> f32 {
    (cx - ax) * (by - ay) - (cy - ay) * (bx - ax)
}

/// Rasterizes a 2D/3D triangle onto the 32-bit color buffer
pub fn rasterize_triangle(
    fb: &mut [u32],
    fb_width: usize,
    fb_height: usize,
    v0: &Vertex,
    v1: &Vertex,
    v2: &Vertex,
) {
    let min_x = v0.x.min(v1.x).min(v2.x).max(0.0) as usize;
    let max_x = (v0.x.max(v1.x).max(v2.x).min((fb_width - 1) as f32)) as usize;
    let min_y = v0.y.min(v1.y).min(v2.y).max(0.0) as usize;
    let max_y = (v0.y.max(v1.y).max(v2.y).min((fb_height - 1) as f32)) as usize;

    if min_x > max_x || min_y > max_y {
        return;
    }

    let area = edge_func(v0.x, v0.y, v1.x, v1.y, v2.x, v2.y);
    if area.abs() < 1e-4 {
        return; // Degenerate triangle
    }

    let inv_area = 1.0 / area;

    for y in min_y..=max_y {
        let py = y as f32 + 0.5;
        let row_offset = y * fb_width;

        for x in min_x..=max_x {
            let px = x as f32 + 0.5;

            let w0 = edge_func(v1.x, v1.y, v2.x, v2.y, px, py);
            let w1 = edge_func(v2.x, v2.y, v0.x, v0.y, px, py);
            let w2 = edge_func(v0.x, v0.y, v1.x, v1.y, px, py);

            let is_inside = if area > 0.0 {
                w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0
            } else {
                w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0
            };

            if is_inside {
                let l0 = (w0 * inv_area).abs();
                let l1 = (w1 * inv_area).abs();
                let l2 = (w2 * inv_area).abs();

                // Barycentric Color Interpolation
                let (r0, g0, b0, a0) = (
                    ((v0.color >> 16) & 0xFF) as f32,
                    ((v0.color >> 8) & 0xFF) as f32,
                    (v0.color & 0xFF) as f32,
                    ((v0.color >> 24) & 0xFF) as f32,
                );
                let (r1, g1, b1, a1) = (
                    ((v1.color >> 16) & 0xFF) as f32,
                    ((v1.color >> 8) & 0xFF) as f32,
                    (v1.color & 0xFF) as f32,
                    ((v1.color >> 24) & 0xFF) as f32,
                );
                let (r2, g2, b2, a2) = (
                    ((v2.color >> 16) & 0xFF) as f32,
                    ((v2.color >> 8) & 0xFF) as f32,
                    (v2.color & 0xFF) as f32,
                    ((v2.color >> 24) & 0xFF) as f32,
                );

                let cr = (l0 * r0 + l1 * r1 + l2 * r2) as u32;
                let cg = (l0 * g0 + l1 * g1 + l2 * g2) as u32;
                let cb = (l0 * b0 + l1 * b1 + l2 * b2) as u32;
                let ca = (l0 * a0 + l1 * a1 + l2 * a2) as u32;

                let src = (ca.min(255) << 24) | (cr.min(255) << 16) | (cg.min(255) << 8) | cb.min(255);
                let dst = fb[row_offset + x];
                fb[row_offset + x] = blend_pixel(src, dst);
            }
        }
    }
}

/// Rasterizes a 2D Gradient Quad (used extensively by Xbox 360 XUI controls and blades)
pub fn rasterize_gradient_quad(
    fb: &mut [u32],
    fb_width: usize,
    fb_height: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    color_top: u32,
    color_bottom: u32,
) {
    let v0 = Vertex { x, y, z: 0.0, color: color_top };
    let v1 = Vertex { x: x + width, y, z: 0.0, color: color_top };
    let v2 = Vertex { x: x + width, y: y + height, z: 0.0, color: color_bottom };
    let v3 = Vertex { x, y: y + height, z: 0.0, color: color_bottom };

    // Quad = 2 Triangles: (v0, v1, v2) and (v2, v3, v0)
    rasterize_triangle(fb, fb_width, fb_height, &v0, &v1, &v2);
    rasterize_triangle(fb, fb_width, fb_height, &v2, &v3, &v0);
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
