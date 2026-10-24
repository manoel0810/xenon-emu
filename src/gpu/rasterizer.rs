//! 2D helpers used by the display's own overlay drawing (not by the GPU emulation).

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
