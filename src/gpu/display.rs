use crate::gpu::font::draw_text;
use minifb::{Key, Window, WindowOptions};
use std::fs::File;
use std::io::Write;

pub const DISPLAY_WIDTH: usize = 1280;
pub const DISPLAY_HEIGHT: usize = 720;

#[derive(Debug, Clone)]
pub struct BladeItem {
    pub title: &'static str,
    pub subtitle: &'static str,
}

#[derive(Debug, Clone)]
pub struct BladeData {
    pub name: &'static str,
    pub primary_color: u32,
    pub secondary_color: u32,
    pub items: Vec<BladeItem>,
}

pub struct GpuDisplay {
    pub width: usize,
    pub height: usize,
    pub framebuffer: Vec<u32>,
    pub window: Window,
    pub frame_count: u64,
    pub active_blade: usize,
    pub selected_item: usize,
    pub key_cooldown: u32,
    pub blades: Vec<BladeData>,
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

        let blades = vec![
            BladeData {
                name: "XBOX LIVE",
                primary_color: 0x00E65C00, // Vibrant Xbox Live Orange
                secondary_color: 0x00993D00,
                items: vec![
                    BladeItem {
                        title: "Xbox Live Marketplace",
                        subtitle: "New Demos, Arcade Games & HD Trailers",
                    },
                    BladeItem {
                        title: "Messages & Chat",
                        subtitle: "Voice Mail, Text & Game Invites (1 New)",
                    },
                    BladeItem {
                        title: "Friends List",
                        subtitle: "3 Friends Online Playing Halo 3 & PGR3",
                    },
                    BladeItem {
                        title: "Account Management",
                        subtitle: "Gamertag, Billing & Xbox Live Gold Subscription",
                    },
                ],
            },
            BladeData {
                name: "GAMES",
                primary_color: 0x00107C10, // Xbox Emerald Green
                secondary_color: 0x000B4D0B,
                items: vec![
                    BladeItem {
                        title: "Play Disc: $flash_dash.xex",
                        subtitle: "Official Xbox 360 Dashboard v2.0.4532.0 (Active)",
                    },
                    BladeItem {
                        title: "Achievements & Gamer Score",
                        subtitle: "18 Games Played | 650 G Unlocked",
                    },
                    BladeItem {
                        title: "Xbox Live Arcade",
                        subtitle: "Geometry Wars, Hexic HD, Pac-Man C.E.",
                    },
                    BladeItem {
                        title: "Game Demos & Trailers",
                        subtitle: "Explore New Xbox 360 High-Definition Releases",
                    },
                ],
            },
            BladeData {
                name: "MEDIA",
                primary_color: 0x000078D7, // Azure Media Blue
                secondary_color: 0x00004578,
                items: vec![
                    BladeItem {
                        title: "Music Player",
                        subtitle: "Play Music from Hard Drive, USB, or Ripped CD",
                    },
                    BladeItem {
                        title: "Pictures & Slideshow",
                        subtitle: "View High-Resolution Photos with Custom Soundtrack",
                    },
                    BladeItem {
                        title: "Video Player",
                        subtitle: "WMV HD, MP4, and DVD Movies (720p 60Hz)",
                    },
                    BladeItem {
                        title: "Windows Media Center",
                        subtitle: "Stream TV, Movies & Photos from Windows PC",
                    },
                ],
            },
            BladeData {
                name: "SYSTEM",
                primary_color: 0x006B2D90, // Royal System Amethyst Purple
                secondary_color: 0x00401B55,
                items: vec![
                    BladeItem {
                        title: "Console Settings",
                        subtitle: "Display (720p HD 60Hz), Audio (Dolby Digital 5.1)",
                    },
                    BladeItem {
                        title: "Family & Parental Controls",
                        subtitle: "Console Safety, ESRB Ratings & Access Limits",
                    },
                    BladeItem {
                        title: "Memory & Storage Units",
                        subtitle: "Corona 16MB NAND Flash, 20GB HDD, USB Storage",
                    },
                    BladeItem {
                        title: "Network & Connection Status",
                        subtitle: "Wired Ethernet / Xenon HLE Network Connected",
                    },
                ],
            },
        ];

        Ok(Self {
            width: DISPLAY_WIDTH,
            height: DISPLAY_HEIGHT,
            framebuffer,
            window,
            frame_count: 0,
            active_blade: 1, // Start on the iconic "GAMES" blade!
            selected_item: 0,
            key_cooldown: 0,
            blades,
        })
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open() && !self.window.is_key_down(Key::Escape)
    }

    /// Handles keyboard navigation across the Blades and menus
    pub fn handle_input(&mut self) {
        if self.key_cooldown > 0 {
            self.key_cooldown -= 1;
            return;
        }

        let mut changed = false;

        // Left / Right (Switch Blades)
        if self.window.is_key_down(Key::Left) || self.window.is_key_down(Key::A) {
            self.active_blade = if self.active_blade == 0 {
                self.blades.len() - 1
            } else {
                self.active_blade - 1
            };
            self.selected_item = 0;
            self.key_cooldown = 12; // 200ms debounce at 60 FPS
            changed = true;
        } else if self.window.is_key_down(Key::Right) || self.window.is_key_down(Key::D) {
            self.active_blade = (self.active_blade + 1) % self.blades.len();
            self.selected_item = 0;
            self.key_cooldown = 12;
            changed = true;
        }

        // Up / Down (Switch Menu items)
        if self.window.is_key_down(Key::Up) || self.window.is_key_down(Key::W) {
            let count = self.blades[self.active_blade].items.len();
            self.selected_item = if self.selected_item == 0 {
                count - 1
            } else {
                self.selected_item - 1
            };
            self.key_cooldown = 10;
            changed = true;
        } else if self.window.is_key_down(Key::Down) || self.window.is_key_down(Key::S) {
            let count = self.blades[self.active_blade].items.len();
            self.selected_item = (self.selected_item + 1) % count;
            self.key_cooldown = 10;
            changed = true;
        }

        if changed {
            println!(
                "  [DASH UI NAV] Active Blade: '{}' | Selected Item: '{}'",
                self.blades[self.active_blade].name,
                self.blades[self.active_blade].items[self.selected_item].title
            );
        }
    }

    /// Renders the legendary authentic Xbox 360 "Blades" Dashboard UI
    pub fn render_blades_dashboard(&mut self, title_id: &str, dash_version: &str) {
        let w = self.width;
        let h = self.height;
        let cur_blade = &self.blades[self.active_blade];

        // 1. Dynamic Flowing Wallpaper with Curved Ambient Glow
        let pulse = ((self.frame_count as f32 * 0.05).sin() * 0.15 + 0.85) as f32;
        let (br, bg, bb) = (
            ((cur_blade.primary_color >> 16) & 0xFF) as f32,
            ((cur_blade.primary_color >> 8) & 0xFF) as f32,
            (cur_blade.primary_color & 0xFF) as f32,
        );

        for y in 0..h {
            let ty = y as f32 / h as f32;
            for x in 0..w {
                let tx = x as f32 / w as f32;
                let wave = ((tx * 6.0 + (self.frame_count as f32 * 0.03)).sin() * 0.08)
                    + ((ty * 4.0 - (self.frame_count as f32 * 0.02)).cos() * 0.06);

                let dist_from_center = ((tx - 0.5) * (tx - 0.5) + (ty - 0.5) * (ty - 0.5)).sqrt();
                let ambient = (1.0 - dist_from_center).max(0.0) * pulse;

                let r = ((12.0 + br * 0.15 * ambient + wave * 10.0) * (1.0 - ty * 0.4)).min(255.0) as u32;
                let g = ((16.0 + bg * 0.15 * ambient + wave * 10.0) * (1.0 - ty * 0.4)).min(255.0) as u32;
                let b = ((20.0 + bb * 0.15 * ambient + wave * 10.0) * (1.0 - ty * 0.4)).min(255.0) as u32;

                self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }

        // 2. Top Header Bar (Carbon Glass)
        for y in 0..70 {
            for x in 0..w {
                let orig = self.framebuffer[y * w + x];
                let r = ((orig >> 16) & 0xFF) / 3;
                let g = ((orig >> 8) & 0xFF) / 3 + 10;
                let b = (orig & 0xFF) / 3 + 12;
                self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }

        // Top Border Line with Glowing Xbox Color
        for x in 0..w {
            self.framebuffer[69 * w + x] = cur_blade.primary_color;
            self.framebuffer[70 * w + x] = 0x00404040;
        }

        // 3. Draw Top-Left Xbox 360 Ring of Light Icon
        let icx = 42usize;
        let icy = 35usize;
        let iradius = 22.0f32;
        for y in (icy - 24)..=(icy + 24) {
            for x in (icx - 24)..=(icx + 24) {
                let dx = x as f32 - icx as f32;
                let dy = y as f32 - icy as f32;
                let dist = (dx * dx + dy * dy).sqrt();

                if dist >= iradius - 4.0 && dist <= iradius {
                    // Quad LED segments: Top-Left is Player 1 (Green LED glowing!)
                    if dx <= 0.0 && dy <= 0.0 {
                        self.framebuffer[y * w + x] = 0x0052D053; // Player 1 Connected (Bright Green)
                    } else {
                        self.framebuffer[y * w + x] = 0x002A302A; // Inactive quad LEDs
                    }
                } else if dist < iradius - 5.0 {
                    // Chrome center jewel
                    let sphere_light = ((1.0 - dist / iradius) * 70.0) as u32;
                    self.framebuffer[y * w + x] = (sphere_light << 16) | (sphere_light << 8) | sphere_light;
                }
            }
        }

        // 4. Draw Blade Tabs along the Top Center
        let tab_start_x = 100;
        let tab_w = 170;
        for (i, b) in self.blades.iter().enumerate() {
            let tx = tab_start_x + i * (tab_w + 14);
            let is_active = i == self.active_blade;

            let tab_color = if is_active {
                b.primary_color
            } else {
                0x0033383E
            };

            // Draw tab pill
            for ty in 16..54 {
                for txx in tx..(tx + tab_w) {
                    if txx < w && ty < h {
                        if is_active {
                            self.framebuffer[ty * w + txx] = tab_color;
                        } else {
                            // Dark translucent pill
                            let orig = self.framebuffer[ty * w + txx];
                            let r = (((orig >> 16) & 0xFF) + ((tab_color >> 16) & 0xFF) / 2) / 2;
                            let g = (((orig >> 8) & 0xFF) + ((tab_color >> 8) & 0xFF) / 2) / 2;
                            let b_val = ((orig & 0xFF) + (tab_color & 0xFF) / 2) / 2;
                            self.framebuffer[ty * w + txx] = (r << 16) | (g << 8) | b_val;
                        }
                    }
                }
            }

            // Tab Text
            let text_color = if is_active { 0x00FFFFFF } else { 0x00A0A8B0 };
            draw_text(
                &mut self.framebuffer,
                w,
                h,
                tx + 22,
                30,
                b.name,
                text_color,
                1,
            );

            // Underline for active tab
            if is_active {
                for txx in tx..(tx + tab_w) {
                    if txx < w {
                        self.framebuffer[55 * w + txx] = 0x00FFFFFF;
                        self.framebuffer[56 * w + txx] = 0x00FFFFFF;
                    }
                }
            }
        }

        // 5. Draw Top-Right Gamertag Profile Card
        let gtx = w - 310;
        // Avatar Box
        for y in 14..56 {
            for x in gtx..(gtx + 42) {
                self.framebuffer[y * w + x] = 0x001B6B1B;
            }
        }
        draw_text(&mut self.framebuffer, w, h, gtx + 12, 28, "X", 0x00FFFFFF, 2);

        // Gamertag name & Gamerscore
        draw_text(&mut self.framebuffer, w, h, gtx + 52, 22, "XenonGuest", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, gtx + 52, 38, "650 G  Online", 0x0000FF7F, 1);

        // Clock
        draw_text(&mut self.framebuffer, w, h, w - 85, 30, "20:25", 0x00E0E0E0, 1);

        // 6. Draw Main Blade Curved Panel
        let blade_panel_x = 100;
        let blade_panel_w = 1080;
        let blade_panel_y = 105;
        let blade_panel_h = 515;

        // Draw curved backdrop with vertical gradient
        for y in blade_panel_y..(blade_panel_y + blade_panel_h) {
            let t = (y - blade_panel_y) as f32 / blade_panel_h as f32;
            let pr = ((cur_blade.primary_color >> 16) & 0xFF) as f32;
            let pg = ((cur_blade.primary_color >> 8) & 0xFF) as f32;
            let pb = (cur_blade.primary_color & 0xFF) as f32;

            let sr = ((cur_blade.secondary_color >> 16) & 0xFF) as f32;
            let sg = ((cur_blade.secondary_color >> 8) & 0xFF) as f32;
            let sb = (cur_blade.secondary_color & 0xFF) as f32;

            let r = (pr * (1.0 - t * 0.7) + sr * t * 0.5) as u32;
            let g = (pg * (1.0 - t * 0.7) + sg * t * 0.5) as u32;
            let b = (pb * (1.0 - t * 0.7) + sb * t * 0.5) as u32;

            for x in blade_panel_x..(blade_panel_x + blade_panel_w) {
                // Curved edge mask
                let rel_x = (x - blade_panel_x) as f32 / blade_panel_w as f32;
                let edge_curve = (rel_x * std::f32::consts::PI).sin().powf(0.08);

                let orig = self.framebuffer[y * w + x];
                let orig_r = (orig >> 16) & 0xFF;
                let orig_g = (orig >> 8) & 0xFF;
                let orig_b = orig & 0xFF;

                let blend_r = ((orig_r as f32 * 0.3) + (r as f32 * 0.7 * edge_curve)) as u32;
                let blend_g = ((orig_g as f32 * 0.3) + (g as f32 * 0.7 * edge_curve)) as u32;
                let blend_b = ((orig_b as f32 * 0.3) + (b as f32 * 0.7 * edge_curve)) as u32;

                self.framebuffer[y * w + x] = (blend_r.min(255) << 16) | (blend_g.min(255) << 8) | blend_b.min(255);
            }
        }

        // Blade Border highlight
        for x in blade_panel_x..(blade_panel_x + blade_panel_w) {
            self.framebuffer[blade_panel_y * w + x] = 0x00FFFFFF;
            self.framebuffer[(blade_panel_y + 1) * w + x] = 0x00A0A0A0;
        }

        // 7. Render 4 Blade Action Cards
        let card_start_y = 135;
        let card_h = 100;
        let card_spacing = 18;

        for (idx, item) in cur_blade.items.iter().enumerate() {
            let cy = card_start_y + idx * (card_h + card_spacing);
            let is_selected = idx == self.selected_item;

            let card_x = blade_panel_x + 35;
            let card_w = blade_panel_w - 70;

            // Card background (glassmorphic pill)
            let card_bg = if is_selected {
                0x002A3528
            } else {
                0x00141A1E
            };

            for y in cy..(cy + card_h) {
                let ty = (y - cy) as f32 / card_h as f32;
                for x in card_x..(card_x + card_w) {
                    let mut r = ((card_bg >> 16) & 0xFF) as f32;
                    let mut g = ((card_bg >> 8) & 0xFF) as f32;
                    let mut b = (card_bg & 0xFF) as f32;

                    // Glossy specular gradient top half
                    if ty < 0.45 {
                        let spec = (1.0 - ty / 0.45) * 45.0;
                        r += spec;
                        g += spec;
                        b += spec;
                    }

                    if is_selected {
                        r = (r + 15.0).min(255.0);
                        g = (g + 35.0).min(255.0);
                        b = (b + 15.0).min(255.0);
                    }

                    self.framebuffer[y * w + x] = ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
                }
            }

            // Outline: If selected, glowing Neon Green border!
            let border_color = if is_selected {
                0x0052D053 // Xbox Neon Green
            } else {
                0x00404C56 // Subtle metallic border
            };

            for x in card_x..(card_x + card_w) {
                self.framebuffer[cy * w + x] = border_color;
                self.framebuffer[(cy + card_h - 1) * w + x] = border_color;
                if is_selected {
                    self.framebuffer[(cy + 1) * w + x] = 0x00FFFFFF;
                }
            }
            for y in cy..(cy + card_h) {
                self.framebuffer[y * w + card_x] = border_color;
                self.framebuffer[y * w + (card_x + card_w - 1)] = border_color;
            }

            // Card Icon box
            let icon_x = card_x + 22;
            let icon_y = cy + 22;
            for iy in icon_y..(icon_y + 56) {
                for ix in icon_x..(icon_x + 56) {
                    self.framebuffer[iy * w + ix] = if is_selected {
                        cur_blade.primary_color
                    } else {
                        0x00222A30
                    };
                }
            }

            // Draw Icon letter (e.g. 1, 2, 3, 4)
            let icon_char = match idx {
                0 => '1',
                1 => '2',
                2 => '3',
                _ => '4',
            };
            draw_text(
                &mut self.framebuffer,
                w,
                h,
                icon_x + 20,
                icon_y + 18,
                &icon_char.to_string(),
                0x00FFFFFF,
                2,
            );

            // Card Text
            let title_color = if is_selected { 0x00FFFFFF } else { 0x00E0E0E0 };
            draw_text(
                &mut self.framebuffer,
                w,
                h,
                card_x + 95,
                cy + 25,
                item.title,
                title_color,
                2, // Big 16px title
            );

            draw_text(
                &mut self.framebuffer,
                w,
                h,
                card_x + 97,
                cy + 58,
                item.subtitle,
                0x00A0B0C0,
                1,
            );

            // Chevron on the right
            let chev_color = if is_selected { 0x0000FF7F } else { 0x00607080 };
            draw_text(
                &mut self.framebuffer,
                w,
                h,
                card_x + card_w - 45,
                cy + 38,
                ">",
                chev_color,
                2,
            );
        }

        // 8. Bottom Navigation Guide Ribbon
        let ribbon_y = h - 68;
        for y in ribbon_y..h {
            for x in 0..w {
                let orig = self.framebuffer[y * w + x];
                let r = ((orig >> 16) & 0xFF) / 4;
                let g = ((orig >> 8) & 0xFF) / 4 + 12;
                let b = (orig & 0xFF) / 4 + 18;
                self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
            }
        }
        for x in 0..w {
            self.framebuffer[ribbon_y * w + x] = 0x0052D053;
        }

        // Controller Button Prompts on Ribbon
        let mut bx = 110;
        let by = ribbon_y + 24;

        // Button (A) Select
        draw_button_circle(&mut self.framebuffer, w, h, bx, by + 4, 0x00108010);
        draw_text(&mut self.framebuffer, w, h, bx - 3, by, "A", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, bx + 16, by, "Select", 0x00FFFFFF, 1);
        bx += 115;

        // Button (B) Back
        draw_button_circle(&mut self.framebuffer, w, h, bx, by + 4, 0x00D01010);
        draw_text(&mut self.framebuffer, w, h, bx - 3, by, "B", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, bx + 16, by, "Back", 0x00FFFFFF, 1);
        bx += 105;

        // Button (X) Dashboard Settings
        draw_button_circle(&mut self.framebuffer, w, h, bx, by + 4, 0x001060D0);
        draw_text(&mut self.framebuffer, w, h, bx - 3, by, "X", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, bx + 16, by, "Settings", 0x00FFFFFF, 1);
        bx += 135;

        // Button (Y) Eject Disc
        draw_button_circle(&mut self.framebuffer, w, h, bx, by + 4, 0x00E0A000);
        draw_text(&mut self.framebuffer, w, h, bx - 3, by, "Y", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, bx + 16, by, "Eject Tray", 0x00FFFFFF, 1);
        bx += 145;

        // Navigation hints
        draw_text(
            &mut self.framebuffer,
            w,
            h,
            bx,
            by,
            "[<] [>] Change Blade (Arrows or A/D)",
            0x0090D0FF,
            1,
        );

        // System & Firmware info banner at bottom right
        let sys_info = format!("Title: {} | v{} | 60 FPS", title_id, dash_version);
        draw_text(
            &mut self.framebuffer,
            w,
            h,
            w - 380,
            by,
            &sys_info,
            0x00788898,
            1,
        );
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

/// Helper to draw a small controller button circle with colored fill
fn draw_button_circle(fb: &mut [u32], w: usize, h: usize, cx: usize, cy: usize, color: u32) {
    let r = 8.0f32;
    for dy in -9..=9 {
        for dx in -9..=9 {
            let px = (cx as i32 + dx) as usize;
            let py = (cy as i32 + dy) as usize;
            if px < w && py < h {
                let dist = ((dx * dx + dy * dy) as f32).sqrt();
                if dist <= r {
                    fb[py * w + px] = color;
                } else if dist <= r + 1.0 {
                    fb[py * w + px] = 0x00FFFFFF; // White rim
                }
            }
        }
    }
}
