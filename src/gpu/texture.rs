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

        let files = [
            ("logo", "xboxLogo.png"),
            ("jewel", "guide_jewel.png"),
            ("live_logo", "livelogo.png"),
            ("ico_console", "ico_32x_console.png"),
            ("ico_network", "ico_32x_network.png"),
            ("ico_parental", "ico_32x_parentalControl.png"),
            ("ico_profiles", "ico_32x_Profiles.png"),
            ("ico_system_set", "ico_32x_systemSet.png"),
            ("ico_game", "ico_64x_Game.png"),
            ("ico_games", "ico_64x_games.png"),
            ("ico_music", "ico_64x_music.png"),
            ("ico_system", "ico_64x_System.png"),
            ("ico_trailers", "ico_64x_Trailers.png"),
            ("art_themes", "themes.jpg"),
            ("art_last_played", "lastPlayed.jpg"),
            ("art_arcade", "myArcadeGames.jpg"),
            ("art_media", "media.jpg"),
            ("art_yak", "yak1.jpg"),
        ];

        for (key, file_name) in &files {
            let p = base.join(file_name);
            if p.exists() {
                if let Some(tex) = Texture::load_file(&p) {
                    textures.insert(key.to_string(), tex);
                }
            }
        }

        Self { textures }
    }

    pub fn get(&self, key: &str) -> Option<&Texture> {
        self.textures.get(key)
    }
}
