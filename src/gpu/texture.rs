use std::collections::HashMap;
use std::path::Path;

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

