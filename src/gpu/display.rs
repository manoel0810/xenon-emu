use minifb::{Key, Window, WindowOptions};
use std::fs::File;
use std::io::Write;

pub const DISPLAY_WIDTH: usize = 1280;
pub const DISPLAY_HEIGHT: usize = 720;

pub struct GpuDisplay {
    pub width: usize,
    pub height: usize,
    pub framebuffer: Vec<u32>,
    pub window: Window,
    pub frame_count: u64,
}

impl GpuDisplay {
    pub fn new(title: &str) -> Result<Self, String> {
        let options = WindowOptions {
            resize: false,
            scale: minifb::Scale::X1,
            ..WindowOptions::default()
        };

        let mut window = Window::new(title, DISPLAY_WIDTH, DISPLAY_HEIGHT, options)
            .map_err(|e| format!("Failed to create window: {:?}", e))?;

        window.set_target_fps(60);

        let framebuffer = vec![0u32; DISPLAY_WIDTH * DISPLAY_HEIGHT];

        Ok(Self {
            width: DISPLAY_WIDTH,
            height: DISPLAY_HEIGHT,
            framebuffer,
            window,
            frame_count: 0,
        })
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open() && !self.window.is_key_down(Key::Escape)
    }

    /// Clears screen with Xbox 360 signature background gradient (Deep Graphite to Xbox Emerald)
    pub fn render_dashboard_splash(
        &mut self,
        _title_id: &str,
        _dash_version: &str,
        _kernel_status: &str,
    ) {
        let w = self.width;
        let h = self.height;

        // 1. Draw smooth radiant gradient
        for y in 0..h {
            let t = y as f32 / h as f32;
            for x in 0..w {
                let dx = (x as f32 - w as f32 / 2.0) / (w as f32 / 2.0);
                let dy = (y as f32 - h as f32 / 2.0) / (h as f32 / 2.0);
                let dist = (dx * dx + dy * dy).sqrt().min(1.0);

                // Xbox 360 Classic Dashboard Aesthetic:
                let r = ((10.0 + (1.0 - dist) * 15.0) * (1.0 - t * 0.3)) as u32;
                let g = ((25.0 + (1.0 - dist) * 110.0 + (1.0 - t) * 20.0)) as u32;
                let b = ((15.0 + (1.0 - dist) * 25.0)) as u32;

                self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }

        // 2. Draw Xbox 360 Ring of Light (Quad-LED Center Sphere)
        let cx = w / 2;
        let cy = h / 2 - 20;
        let ring_radius = 100.0f32;
        let ring_thickness = 14.0f32;

        let pulse = ((self.frame_count as f32 * 0.08).sin() * 0.2 + 0.8) as f32;

        for y in (cy - 130)..=(cy + 130) {
            for x in (cx - 130)..=(cx + 130) {
                let dx = x as f32 - cx as f32;
                let dy = y as f32 - cy as f32;
                let dist = (dx * dx + dy * dy).sqrt();

                // Outer chrome / silver ring
                if dist >= ring_radius - ring_thickness && dist <= ring_radius + 2.0 {
                    let angle = dy.atan2(dx);
                    let gap = (angle.sin().abs() < 0.07) || (angle.cos().abs() < 0.07);

                    if !gap {
                        // Glowing Xbox Emerald LED (Player 1 indicator)
                        let green_intensity = (230.0 * pulse) as u32;
                        self.framebuffer[y * w + x] = (0x28 << 16) | (green_intensity << 8) | 0x20;
                    } else {
                        self.framebuffer[y * w + x] = 0x00141614;
                    }
                }

                // Inner sphere reflection (Xbox 360 metallic jewel)
                if dist < ring_radius - ring_thickness - 4.0 {
                    let sphere_shade = ((1.0 - dist / (ring_radius - 18.0)) * 55.0) as u32;
                    let existing = self.framebuffer[y * w + x];
                    let r = ((existing >> 16) & 0xFF).saturating_add(sphere_shade);
                    let g = ((existing >> 8) & 0xFF).saturating_add(sphere_shade + 12);
                    let b = (existing & 0xFF).saturating_add(sphere_shade);
                    self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
                }
            }
        }

        // 3. Draw Top Dashboard Title Bar
        for y in 0..65 {
            for x in 0..w {
                let existing = self.framebuffer[y * w + x];
                let r = (((existing >> 16) & 0xFF) / 3) as u32;
                let g = (((existing >> 8) & 0xFF) / 3 + 25) as u32;
                let b = ((existing & 0xFF) / 3) as u32;
                self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }

        // 4. Draw Bottom Dashboard Navigation Ribbon
        for y in (h - 75)..h {
            for x in 0..w {
                let existing = self.framebuffer[y * w + x];
                let r = (((existing >> 16) & 0xFF) / 4) as u32;
                let g = (((existing >> 8) & 0xFF) / 4 + 35) as u32;
                let b = ((existing & 0xFF) / 4) as u32;
                self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }
    }

    /// Blits the framebuffer to the OS window and handles Windows messages
    pub fn update(&mut self) -> Result<(), String> {
        self.frame_count += 1;
        self.window
            .update_with_buffer(&self.framebuffer, self.width, self.height)
            .map_err(|e| format!("Window update error: {:?}", e))
    }

    /// Saves the current 1280x720 framebuffer as a standard 24-bit BMP image
    pub fn save_bmp(&self, path: &str) -> Result<(), String> {
        let mut file = File::create(path).map_err(|e| format!("Create file failed: {:?}", e))?;
        let w = self.width as u32;
        let h = self.height as u32;

        let row_size = (w * 3 + 3) & !3; // Align to 4 bytes
        let pixel_array_size = row_size * h;
        let file_size = 54 + pixel_array_size;

        // BMP Header
        let mut header = [0u8; 54];
        header[0] = b'B';
        header[1] = b'M';
        header[2..6].copy_from_slice(&file_size.to_le_bytes());
        header[10..14].copy_from_slice(&54u32.to_le_bytes()); // Offset to pixel data
        header[14..18].copy_from_slice(&40u32.to_le_bytes()); // DIB Header size
        header[18..22].copy_from_slice(&w.to_le_bytes());
        header[22..26].copy_from_slice(&h.to_le_bytes());
        header[26..28].copy_from_slice(&1u16.to_le_bytes());  // Color planes
        header[28..30].copy_from_slice(&24u16.to_le_bytes()); // 24 bpp
        header[34..38].copy_from_slice(&pixel_array_size.to_le_bytes());

        file.write_all(&header).map_err(|e| format!("Write header failed: {:?}", e))?;

        // Pixels (BMP is bottom-up, BGR format)
        let mut row = vec![0u8; row_size as usize];
        for y in (0..self.height).rev() {
            for x in 0..self.width {
                let pixel = self.framebuffer[y * self.width + x];
                let r = ((pixel >> 16) & 0xFF) as u8;
                let g = ((pixel >> 8) & 0xFF) as u8;
                let b = (pixel & 0xFF) as u8;
                row[x * 3] = b;
                row[x * 3 + 1] = g;
                row[x * 3 + 2] = r;
            }
            file.write_all(&row).map_err(|e| format!("Write row failed: {:?}", e))?;
        }

        Ok(())
    }
}
