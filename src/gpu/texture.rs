// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/texture_util.cc       TiledOffset2D (Xenos macro/micro tiling untile)
//   src/xenia/gpu/xenos.h              TextureFormat, TextureClamp, TextureFilter, xe_gpu_texture_fetch_t
//   src/xenia/gpu/spirv_shader_translator_fetch.cc texture fetch coordinate unnormalization, filtering, swizzle
//
//! Xenos texture untile, formats and sampling for tfetch instructions (Phase 4.4).
//! Also includes legacy Texture/TextureAtlas for fallback visual presentation until Phase 4.5.

use std::collections::HashMap;
use std::path::Path;

use super::shader_env::{TextureFetchConstant, TextureSampler};
use super::shader_interp::Vec4;
use super::ucode::FetchInstruction;
use super::xenos::guest_endian_swap;
use crate::memory::MemoryBus;

/// Texture formats sampled without a decoder, one bit per TextureFormat (reported once).
static UNSUPPORTED_FORMATS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Untile offset for 2D textures according to Xenos micro/macro tiling.
/// Returns the element offset (in pixels for uncompressed formats, or in 4x4 blocks for compressed formats).
/// Multiply by bytes_per_element (1 << log2_bpp) to get the byte offset.
///
/// Ported from Xenia Canary `src/xenia/gpu/texture_util.cc` (TiledOffset2D).
#[inline(always)]
pub fn tiled_offset_2d(x: u32, y: u32, width_aligned: u32, log2_bpp: u32) -> u32 {
    let macro_val = ((x >> 5) + (y >> 5) * (width_aligned >> 5)) << (log2_bpp + 7);
    let micro = ((x & 7) + ((y & 6) << 2)) << log2_bpp;
    let offset = macro_val
        + ((micro & !15) << 1)
        + (micro & 15)
        + ((y & 8) << (3 + log2_bpp))
        + ((y & 1) << 4);
    let bank = ((((y & 8) >> 2) + ((x >> 3) & 3)) & 3) << 6;
    (((offset & !511) << 3)
        + ((offset & 448) << 2)
        + (offset & 63)
        + ((y & 16) << 7)
        + bank) >> log2_bpp
}

/// Clamps coordinate `t` into `[0..size]` according to `clamp_mode` (TextureClamp).
/// Returns `None` if coordinate falls outside and clamp mode specifies border color.
fn clamp_coord(t: i32, size: u32, clamp_mode: u32) -> Option<u32> {
    let s = size as i32;
    if s <= 0 {
        return Some(0);
    }
    match clamp_mode {
        // 0: Wrap
        0 => Some(t.rem_euclid(s) as u32),
        // 1: Mirror
        1 => {
            let m = t.div_euclid(s).abs() % 2;
            let r = t.rem_euclid(s);
            Some(if m == 0 { r as u32 } else { (s - 1 - r) as u32 })
        }
        // 2: ClampToEdge (or 6: MirrorOnceClampToEdge fallback)
        2 | 6 => Some(t.clamp(0, s - 1) as u32),
        // 3: BorderColor (or 4, 5: MirrorOnce border)
        3 | 4 | 5 => {
            if t < 0 || t >= s {
                None
            } else {
                Some(t as u32)
            }
        }
        _ => Some(t.clamp(0, s - 1) as u32),
    }
}

/// BorderColor: k_AGBR_Black, k_AGBR_White; the two YCbCr blacks are treated as black.
fn border_color_value(border_type: u32) -> Vec4 {
    match border_type {
        1 => [1.0, 1.0, 1.0, 1.0],
        _ => [0.0, 0.0, 0.0, 0.0],
    }
}

/// A `bits`-wide texel component through TextureSign and num_format (deviation: kGamma is read as unsigned, without
/// Xenia's piecewise-linear degamma).
fn texel_component(raw: u32, bits: u32, sign: u32, integer: bool) -> f32 {
    let max = ((1u64 << bits) - 1) as f32;
    let signed = || ((raw << (32 - bits)) as i32 >> (32 - bits)) as f32;
    match (sign, integer) {
        (1, true) => signed(),
        (_, true) => raw as f32,
        (1, false) => (signed() / (max / 2.0).floor()).max(-1.0),
        (2, false) => raw as f32 / max * 2.0 - 1.0,
        _ => raw as f32 / max,
    }
}

/// Emulated Xenos Texture Sampler accessing guest physical memory through the MemoryBus.
pub struct GpuTextureSampler<'a> {
    pub bus: &'a MemoryBus,
    /// Every fetch with its result, only when built with `recording` (diagnostics).
    pub fetches: Vec<(TextureFetchConstant, Vec4, Vec4)>,
    record: bool,
}

impl<'a> GpuTextureSampler<'a> {
    pub fn new(bus: &'a MemoryBus) -> Self {
        Self { bus, fetches: Vec::new(), record: false }
    }

    /// A sampler that keeps every fetch in `fetches`.
    pub fn recording(bus: &'a MemoryBus) -> Self {
        Self { record: true, ..Self::new(bus) }
    }

    /// Samples a single texel at integer pixel coordinates (x_coord, y_coord).
    pub fn sample_texel(&self, constant: &TextureFetchConstant, x_coord: i32, y_coord: i32, width: u32, height: u32) -> Vec4 {
        let Some(x) = clamp_coord(x_coord, width, constant.clamp_x()) else {
            return border_color_value(constant.border_color());
        };
        let Some(y) = clamp_coord(y_coord, height, constant.clamp_y()) else {
            return border_color_value(constant.border_color());
        };

        let base = constant.base_address() as u64;
        let fmt = constant.format();
        let integer = constant.num_format() == 1;
        let raw_components = match fmt {
            // Format 6: k_8_8_8_8 (32 bpp = 4 bytes)
            6 => {
                let byte_offset = if constant.tiled() {
                    (tiled_offset_2d(x, y, constant.pitch_texels(width), 2) as u64) * 4
                } else {
                    let row_pitch = constant.linear_pitch_bytes(width, 4) as u64;
                    (y as u64) * row_pitch + (x as u64) * 4
                };
                let raw = self.bus.read_u32(base + byte_offset).unwrap_or(0);
                let swapped = guest_endian_swap(raw, constant.endianness());
                std::array::from_fn(|c| texel_component((swapped >> (8 * c)) & 0xFF, 8, constant.sign(c), integer))
            }
            // Format 2: k_8 (8 bpp = 1 byte)
            2 => {
                let byte_offset = if constant.tiled() {
                    tiled_offset_2d(x, y, constant.pitch_texels(width), 0) as u64
                } else {
                    let row_pitch = constant.linear_pitch_bytes(width, 1) as u64;
                    (y as u64) * row_pitch + (x as u64)
                };
                let raw = self.bus.read_u8(base + byte_offset).unwrap_or(0);
                let v = texel_component(raw as u32, 8, constant.sign(0), integer);
                [v, v, v, 1.0]
            }
            // Format 25: k_16_16 (32 bpp = 4 bytes: two 16-bit)
            25 => {
                let byte_offset = if constant.tiled() {
                    (tiled_offset_2d(x, y, constant.pitch_texels(width), 2) as u64) * 4
                } else {
                    let row_pitch = constant.linear_pitch_bytes(width, 4) as u64;
                    (y as u64) * row_pitch + (x as u64) * 4
                };
                let raw = self.bus.read_u32(base + byte_offset).unwrap_or(0);
                let swapped = guest_endian_swap(raw, constant.endianness());
                [
                    texel_component(swapped & 0xFFFF, 16, constant.sign(0), integer),
                    texel_component(swapped >> 16, 16, constant.sign(1), integer),
                    0.0,
                    1.0,
                ]
            }
            // Fallback for other formats: opaque white (each format reported once)
            other => {
                let bit = 1u64 << (other & 63);
                if UNSUPPORTED_FORMATS.fetch_or(bit, std::sync::atomic::Ordering::Relaxed) & bit == 0 {
                    println!("  [TEXTURE] format {} is not decoded yet: sampled as opaque white", other);
                }
                [1.0, 1.0, 1.0, 1.0]
            }
        };

        // Apply constant component swizzle: 0=R, 1=G, 2=B, 3=A, 4=0.0, 5=1.0
        let mut swizzled = [0.0f32; 4];
        for (c, out) in swizzled.iter_mut().enumerate() {
            *out = match constant.swizzle(c) {
                0 => raw_components[0],
                1 => raw_components[1],
                2 => raw_components[2],
                3 => raw_components[3],
                4 => 0.0,
                5 => 1.0,
                _ => 0.0,
            };
        }

        // Apply exp_adjust
        let exp = constant.exp_adjust();
        if exp != 0 {
            let factor = 2f32.powi(exp);
            for v in &mut swizzled {
                *v *= factor;
            }
        }

        swizzled
    }
}

impl TextureSampler for GpuTextureSampler<'_> {
    fn sample(&mut self, constant: &TextureFetchConstant, op: &FetchInstruction, coords: Vec4) -> Vec4 {
        let (w, h) = constant.size_2d();
        if w == 0 || h == 0 {
            return [0.0; 4];
        }

        // Coordinate calculation (normalized vs unnormalized, plus instruction offset)
        let (mut u, mut v) = if op.tf_unnormalized_coordinates() {
            (coords[0], coords[1])
        } else {
            (coords[0] * w as f32, coords[1] * h as f32)
        };
        u += op.tf_offset_x();
        v += op.tf_offset_y();

        let filter = if op.tf_mag_filter() == 3 { constant.mag_filter() } else { op.tf_mag_filter() };
        let result = if filter == 0 {
            // Point / Nearest
            self.sample_texel(constant, u.floor() as i32, v.floor() as i32, w, h)
        } else {
            // Bilinear
            let u_prime = u - 0.5;
            let v_prime = v - 0.5;
            let x0 = u_prime.floor() as i32;
            let y0 = v_prime.floor() as i32;
            let x1 = x0 + 1;
            let y1 = y0 + 1;
            let fx = u_prime - x0 as f32;
            let fy = v_prime - y0 as f32;

            let t00 = self.sample_texel(constant, x0, y0, w, h);
            let t10 = self.sample_texel(constant, x1, y0, w, h);
            let t01 = self.sample_texel(constant, x0, y1, w, h);
            let t11 = self.sample_texel(constant, x1, y1, w, h);

            let w00 = (1.0 - fx) * (1.0 - fy);
            let w10 = fx * (1.0 - fy);
            let w01 = (1.0 - fx) * fy;
            let w11 = fx * fy;

            let mut out = [0.0f32; 4];
            for c in 0..4 {
                out[c] = t00[c] * w00 + t10[c] * w10 + t01[c] * w01 + t11[c] * w11;
            }
            out
        };

        if self.record {
            self.fetches.push((*constant, coords, result));
        }
        result
    }
}

#[derive(Debug, Clone)]
pub struct Texture {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>, // 0xAARRGGBB format
}

impl Texture {
    pub fn new(width: usize, height: usize, pixels: Vec<u32>) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }

    pub fn load_file<P: AsRef<Path>>(path: P) -> Option<Self> {
        let dyn_img = image::open(path).ok()?;
        let rgba = dyn_img.to_rgba8();
        let (w, h) = rgba.dimensions();
        let mut pixels = Vec::with_capacity((w * h) as usize);

        for pixel in rgba.pixels() {
            let r = pixel[0] as u32;
            let g = pixel[1] as u32;
            let b = pixel[2] as u32;
            let a = pixel[3] as u32;
            let argb = (a << 24) | (r << 16) | (g << 8) | b;
            pixels.push(argb);
        }

        Some(Self {
            width: w as usize,
            height: h as usize,
            pixels,
        })
    }

    /// Bilinear texture filtering given normalized coordinates (u, v) in 0.0..1.0
    pub fn sample_bilinear(&self, u: f32, v: f32) -> u32 {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return 0;
        }

        let u_clamped = u.clamp(0.0, 1.0) * (self.width - 1) as f32;
        let v_clamped = v.clamp(0.0, 1.0) * (self.height - 1) as f32;

        let x0 = u_clamped.floor() as usize;
        let y0 = v_clamped.floor() as usize;
        let x1 = (x0 + 1).min(self.width - 1);
        let y1 = (y0 + 1).min(self.height - 1);

        let fx = u_clamped - x0 as f32;
        let fy = v_clamped - y0 as f32;
        let ifx = 1.0 - fx;
        let ify = 1.0 - fy;

        let p00 = self.pixels[y0 * self.width + x0];
        let p10 = self.pixels[y0 * self.width + x1];
        let p01 = self.pixels[y1 * self.width + x0];
        let p11 = self.pixels[y1 * self.width + x1];

        let w00 = ifx * ify;
        let w10 = fx * ify;
        let w01 = ifx * fy;
        let w11 = fx * fy;

        let a = (((p00 >> 24) & 0xFF) as f32 * w00
            + ((p10 >> 24) & 0xFF) as f32 * w10
            + ((p01 >> 24) & 0xFF) as f32 * w01
            + ((p11 >> 24) & 0xFF) as f32 * w11) as u32;

        let r = (((p00 >> 16) & 0xFF) as f32 * w00
            + ((p10 >> 16) & 0xFF) as f32 * w10
            + ((p01 >> 16) & 0xFF) as f32 * w01
            + ((p11 >> 16) & 0xFF) as f32 * w11) as u32;

        let g = (((p00 >> 8) & 0xFF) as f32 * w00
            + ((p10 >> 8) & 0xFF) as f32 * w10
            + ((p01 >> 8) & 0xFF) as f32 * w01
            + ((p11 >> 8) & 0xFF) as f32 * w11) as u32;

        let b = ((p00 & 0xFF) as f32 * w00
            + (p10 & 0xFF) as f32 * w10
            + (p01 & 0xFF) as f32 * w01
            + (p11 & 0xFF) as f32 * w11) as u32;

        (a.min(255) << 24) | (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
    }

    /// Point/nearest-neighbor texture sampling
    pub fn sample_nearest(&self, u: f32, v: f32) -> u32 {
        if self.width == 0 || self.height == 0 || self.pixels.is_empty() {
            return 0;
        }
        let tx = ((u.clamp(0.0, 1.0) * (self.width - 1) as f32) as usize).min(self.width - 1);
        let ty = ((v.clamp(0.0, 1.0) * (self.height - 1) as f32) as usize).min(self.height - 1);
        self.pixels[ty * self.width + tx]
    }

    /// Blits texture onto target framebuffer at (dx, dy) with per-pixel alpha blending
    pub fn draw(&self, fb: &mut [u32], fb_w: usize, fb_h: usize, dx: i32, dy: i32) {
        self.draw_scaled(fb, fb_w, fb_h, dx, dy, self.width, self.height, 1.0);
    }

    /// Blits texture scaled to (target_w, target_h) with global alpha modulation
    pub fn draw_scaled(
        &self,
        fb: &mut [u32],
        fb_w: usize,
        fb_h: usize,
        dx: i32,
        dy: i32,
        target_w: usize,
        target_h: usize,
        global_alpha: f32,
    ) {
        if target_w == 0 || target_h == 0 || global_alpha <= 0.001 {
            return;
        }

        let sx_step = (self.width as f32) / (target_w as f32);
        let sy_step = (self.height as f32) / (target_h as f32);

        for py in 0..target_h {
            let out_y = dy + py as i32;
            if out_y < 0 || out_y >= fb_h as i32 {
                continue;
            }

            let src_y = ((py as f32 * sy_step) as usize).min(self.height - 1);
            let row_offset = (out_y as usize) * fb_w;
            let src_row = src_y * self.width;

            for px in 0..target_w {
                let out_x = dx + px as i32;
                if out_x < 0 || out_x >= fb_w as i32 {
                    continue;
                }

                let src_x = ((px as f32 * sx_step) as usize).min(self.width - 1);
                let src_pixel = self.pixels[src_row + src_x];

                let src_a = ((src_pixel >> 24) & 0xFF) as f32 * global_alpha;
                if src_a <= 1.0 {
                    continue;
                }

                let idx = row_offset + out_x as usize;
                if src_a >= 254.0 {
                    fb[idx] = src_pixel;
                } else {
                    let alpha = src_a / 255.0;
                    let inv_alpha = 1.0 - alpha;

                    let dst_pixel = fb[idx];
                    let dr = ((dst_pixel >> 16) & 0xFF) as f32;
                    let dg = ((dst_pixel >> 8) & 0xFF) as f32;
                    let db = (dst_pixel & 0xFF) as f32;

                    let sr = ((src_pixel >> 16) & 0xFF) as f32;
                    let sg = ((src_pixel >> 8) & 0xFF) as f32;
                    let sb = (src_pixel & 0xFF) as f32;

                    let out_r = (sr * alpha + dr * inv_alpha) as u32;
                    let out_g = (sg * alpha + dg * inv_alpha) as u32;
                    let out_b = (sb * alpha + db * inv_alpha) as u32;

                    fb[idx] = (out_r.min(255) << 16) | (out_g.min(255) << 8) | out_b.min(255);
                }
            }
        }
    }
}

/// Preloaded cache of authentic Xbox 360 Dashboard textures
pub struct TextureAtlas {
    pub textures: HashMap<String, Texture>,
}

impl TextureAtlas {
    pub fn load_from_dir<P: AsRef<Path>>(dir: P) -> Self {
        let mut textures = HashMap::new();
        let base = dir.as_ref();

        let mut candidate_dirs = vec![
            base.to_path_buf(),
            base.join("named"),
            Path::new("../extracted_xui/named").to_path_buf(),
            Path::new("extracted_xui/named").to_path_buf(),
            Path::new("../../extracted_xui/named").to_path_buf(),
            Path::new(r"D:\XBOX 360 - Firmeware Emulator\extracted_xui\named").to_path_buf(),
            Path::new("../extracted_xui").to_path_buf(),
            Path::new("extracted_xui").to_path_buf(),
        ];

        // 1. Scan directories for all valid PNG and JPG files
        for dir_path in candidate_dirs.drain(..) {
            if !dir_path.is_dir() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(&dir_path) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
                        if ext == "png" || ext == "jpg" || ext == "jpeg" {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                if !textures.contains_key(stem) {
                                    if let Some(tex) = Texture::load_file(&path) {
                                        textures.insert(stem.to_string(), tex.clone());
                                        textures.insert(format!("{}.{}", stem, ext), tex);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Standard Aliases
        let aliases = [
            ("logo", "xboxLogo"),
            ("jewel", "guide_jewel"),
            ("live_logo", "livelogo_hor"),
            ("ico_console", "ico_32x_console"),
            ("ico_network", "ico_32x_network"),
            ("ico_parental", "ico_32x_parentalControl"),
            ("ico_profiles", "ico_32x_Profiles"),
            ("ico_system_set", "ico_32x_systemSet"),
            ("ico_game", "ico_64x_Game"),
            ("ico_games", "ico_64x_games"),
            ("ico_music", "ico_64x_music"),
            ("ico_system", "ico_64x_System"),
            ("ico_trailers", "ico_64x_Trailers"),
            ("art_themes", "themes"),
            ("art_last_played", "lastPlayed"),
            ("art_arcade", "myArcadeGames"),
            ("art_media", "media"),
            ("art_yak", "yak1"),
        ];

        for (alias, target) in &aliases {
            if let Some(tex) = textures.get(*target).cloned() {
                textures.insert(alias.to_string(), tex);
            }
        }

        Self { textures }
    }

    pub fn get(&self, key: &str) -> Option<&Texture> {
        self.textures.get(key)
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiled_offset_2d_matches_xenia_for_1x1_and_framebuffer() {
        // 1x1 32-bpp texture: aligned to 32 elements width
        assert_eq!(tiled_offset_2d(0, 0, 32, 2), 0);
        // Framebuffer 1280x720 32-bpp (width 1280): (0, 0) is 0
        assert_eq!(tiled_offset_2d(0, 0, 1280, 2), 0);
        // (1, 0) is element 1
        assert_eq!(tiled_offset_2d(1, 0, 1280, 2), 1);
        // (0, 1) is element 4 (staggered by 16 bytes = 4 dwords in 8x8 microtile)
        assert_eq!(tiled_offset_2d(0, 1, 1280, 2), 4);
    }

    /// XE_GPU_MAKE_TEXTURE_SWIZZLE(X, Y, Z, W).
    const RGBA: u32 = 0 | (1 << 3) | (2 << 6) | (3 << 9);

    #[test]
    fn format_6_unpacks_with_endian_swap_and_swizzle() {
        let mut bus = MemoryBus::new();
        // Base address 0x1000, 1x1 texture, format 6 (8_8_8_8), endian 2 (k8in32)
        // Memory has ARGB: byte0=0x11, byte1=0x22, byte2=0x33, byte3=0x44 (raw dword 0x44332211)
        bus.write_u32(0x1000, 0x4433_2211).unwrap();
        let mut words = [0u32; 6];
        words[0] = (2 << 0) | (2 << 10) | (2 << 13); // 2D, clamp_x=2, clamp_y=2
        words[1] = (6 << 0) | (2 << 6) | (1 << 12);   // format 6, endian 2, base 0x1000 (1 << 12)
        words[2] = 0;                                 // size 1x1 (0+1, 0+1)
        words[3] = RGBA << 1;                         // swizzle in dword_3 bits 1..12
        let constant = TextureFetchConstant { words };
        let sampler = GpuTextureSampler::new(&bus);
        let texel = sampler.sample_texel(&constant, 0, 0, 1, 1);
        assert!((texel[0] - 0x11 as f32 / 255.0).abs() < 1e-4);
        assert!((texel[1] - 0x22 as f32 / 255.0).abs() < 1e-4);
        assert!((texel[2] - 0x33 as f32 / 255.0).abs() < 1e-4);
        assert!((texel[3] - 0x44 as f32 / 255.0).abs() < 1e-4);
        // swizzle BGR1, exp_adjust -1, x signed, y biased
        let mut words = constant.words;
        words[0] |= (1 << 2) | (2 << 4);
        words[3] = ((2 | (1 << 3) | (0 << 6) | (5 << 9)) << 1) | (((-1i32 as u32) & 0x3F) << 13);
        let constant = TextureFetchConstant { words };
        assert_eq!((constant.exp_adjust(), constant.swizzle(3), constant.sign(0), constant.sign(1)), (-1, 5, 1, 2));
        let texel = sampler.sample_texel(&constant, 0, 0, 1, 1);
        let expected = [0x33 as f32 / 255.0, 0x22 as f32 / 255.0 * 2.0 - 1.0, 0x11 as f32 / 127.0, 1.0].map(|v| v * 0.5);
        for c in 0..4 {
            assert!((texel[c] - expected[c]).abs() < 1e-4, "{:?} {:?}", texel, expected);
        }
    }

    #[test]
    fn clamping_modes_wrap_and_clamp_to_edge() {
        assert_eq!(clamp_coord(-1, 10, 2), Some(0));     // ClampToEdge
        assert_eq!(clamp_coord(10, 10, 2), Some(9));
        assert_eq!(clamp_coord(-1, 10, 0), Some(9));     // Wrap
        assert_eq!(clamp_coord(11, 10, 0), Some(1));
        assert_eq!(clamp_coord(-1, 10, 3), None);        // BorderColor
    }

    #[test]
    fn bilinear_filtering_interpolates_subtexels() {
        let mut bus = MemoryBus::new();
        // 2x1 linear texture: pixel 0 = black (0), pixel 1 = white (0xFFFFFFFF)
        bus.write_u32(0x1000, 0x0000_0000).unwrap();
        bus.write_u32(0x1004, 0xFFFF_FFFF).unwrap();
        let mut words = [0u32; 6];
        words[0] = (2 << 0) | (2 << 10) | (2 << 13); // 2D, clamp_x=2, clamp_y=2, linear (tiled=0)
        words[1] = (6 << 0) | (2 << 6) | (1 << 12);   // format 6, endian 2, base 0x1000
        words[2] = 1;                                 // size 2x1 (w=2, h=1)
        words[3] = (RGBA << 1) | (1 << 19);          // swizzle RGBA; mag_filter=1 (bilinear)
        let constant = TextureFetchConstant { words };
        let mut sampler = GpuTextureSampler::new(&bus);
        let mut op = FetchInstruction::default();
        op.0[1] = 3 << 12; // mag_filter=3 (use constant)
        let result = sampler.sample(&constant, &op, [0.5, 0.5, 0.0, 0.0]);
        // Pixel 0 is at u_texel=0.5, pixel 1 is at u_texel=1.5
        // At normalized u=0.5, u_texel = 1.0 (exact midpoint) -> 0.5 interpolation!
        assert!((result[0] - 0.5).abs() < 0.05);
    }
}

