use crate::gpu::font::draw_text;
use crate::gpu::rasterizer::rasterize_rounded_card;
use crate::kernel::input::{
    GamepadManager, XINPUT_GAMEPAD_A, XINPUT_GAMEPAD_B, XINPUT_GAMEPAD_DPAD_DOWN,
    XINPUT_GAMEPAD_DPAD_LEFT, XINPUT_GAMEPAD_DPAD_RIGHT, XINPUT_GAMEPAD_DPAD_UP,
    XINPUT_GAMEPAD_LEFT_SHOULDER, XINPUT_GAMEPAD_RIGHT_SHOULDER,
};
use minifb::{Key, Window, WindowOptions};
use std::fs::File;
use std::io::Write;

pub const DISPLAY_WIDTH: usize = 1280;
pub const DISPLAY_HEIGHT: usize = 720;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    BootAnimation,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Blades,
    XuiDirect,
}

#[derive(Debug, Clone)]
pub struct BladeItem {
    pub title: String,
    pub subtitle: String,
}

#[derive(Debug, Clone)]
pub struct BladeData {
    pub name: String,
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
    pub gamepad_mgr: GamepadManager,
    pub mode: DisplayMode,
    pub boot_frame: u32,
    pub view_mode: ViewMode,
    pub xui_scenes: Vec<crate::loader::XurScene>,
    pub xus_table: Option<crate::loader::XusTable>,
    pub current_scene_idx: usize,
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

        let item = |t: &str, s: &str| BladeItem {
            title: t.to_string(),
            subtitle: s.to_string(),
        };

        let blades = vec![
            BladeData {
                name: "XBOX LIVE".to_string(),
                primary_color: 0x00E65C00, // Vibrant Xbox Live Orange
                secondary_color: 0x00993D00,
                items: vec![
                    item("Xbox Live Marketplace", "New Demos, Arcade Games & HD Trailers"),
                    item("Messages & Chat", "Voice Mail, Text & Game Invites (1 New)"),
                    item("Friends List", "3 Friends Online Playing Halo 3 & PGR3"),
                    item("Account Management", "Gamertag, Billing & Xbox Live Gold Subscription"),
                ],
            },
            BladeData {
                name: "GAMES".to_string(),
                primary_color: 0x00107C10, // Xbox Emerald Green
                secondary_color: 0x000B4D0B,
                items: vec![
                    item("Play Disc: $flash_dash.xex", "Official Xbox 360 Dashboard v2.0.4532.0 (Active)"),
                    item("Achievements & Gamer Score", "18 Games Played | 650 G Unlocked"),
                    item("Xbox Live Arcade", "Geometry Wars, Hexic HD, Pac-Man C.E."),
                    item("Game Demos & Trailers", "Explore New Xbox 360 High-Definition Releases"),
                ],
            },
            BladeData {
                name: "MEDIA".to_string(),
                primary_color: 0x000078D7, // Azure Media Blue
                secondary_color: 0x00004578,
                items: vec![
                    item("Music Player", "Play Music from Hard Drive, USB, or Ripped CD"),
                    item("Pictures & Slideshow", "View High-Resolution Photos with Custom Soundtrack"),
                    item("Video Player", "WMV HD, MP4, and DVD Movies (720p 60Hz)"),
                    item("Windows Media Center", "Stream TV, Movies & Photos from Windows PC"),
                ],
            },
            BladeData {
                name: "SYSTEM".to_string(),
                primary_color: 0x006B2D90, // Royal System Amethyst Purple
                secondary_color: 0x00401B55,
                items: vec![
                    item("Console Settings", "Display (720p HD 60Hz), Audio (Dolby Digital 5.1)"),
                    item("Family & Parental Controls", "Console Safety, ESRB Ratings & Access Limits"),
                    item("Memory & Storage Units", "Corona 16MB NAND Flash, 20GB HDD, USB Storage"),
                    item("Network & Connection Status", "Wired Ethernet / Xenon HLE Network Connected"),
                ],
            },
        ];

        let gamepad_mgr = GamepadManager::new();

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
            gamepad_mgr,
            mode: DisplayMode::BootAnimation,
            boot_frame: 0,
            view_mode: ViewMode::Blades,
            xui_scenes: Vec::new(),
            xus_table: None,
            current_scene_idx: 0,
        })
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open() && !self.window.is_key_down(Key::Escape)
    }

    /// Handles both physical USB Xbox 360 controller and keyboard navigation
    pub fn handle_input(&mut self) {
        if self.key_cooldown > 0 {
            self.key_cooldown -= 1;
            return;
        }

        // In Boot Animation mode, only allow skip after 60 frames (1 second minimum)
        // to prevent terminal Enter bleed-through when launched via RODAR_EMULADOR.bat
        if self.mode == DisplayMode::BootAnimation {
            let mut skip = false;
            if self.boot_frame >= 60 {
                if let Some(state) = self.gamepad_mgr.poll(0) {
                    if (state.gamepad.w_buttons & XINPUT_GAMEPAD_A) != 0 {
                        skip = true;
                    }
                }
                if self.window.is_key_down(Key::Space) || self.window.is_key_down(Key::Escape) {
                    skip = true;
                }
            }

            if skip {
                println!("  [BOOTANIM] Botao de Skip pressionado -> Transicao suave para a Dashboard!");
                self.mode = DisplayMode::Dashboard;
                self.key_cooldown = 20;
            }
            return;
        }

        // Toggle between Blades and Direct Microsoft XUI Scene Graph view (.xur)
        let mut toggle_view = false;
        let mut cycle_scene = false;

        if let Some(state) = self.gamepad_mgr.poll(0) {
            let btns = state.gamepad.w_buttons;
            if (btns & crate::kernel::input::XINPUT_GAMEPAD_Y) != 0 {
                toggle_view = true;
            }
            if (btns & crate::kernel::input::XINPUT_GAMEPAD_X) != 0 {
                cycle_scene = true;
            }
        }

        if self.window.is_key_down(Key::Tab) {
            toggle_view = true;
        }

        if self.window.is_key_down(Key::Key1)
            || self.window.is_key_down(Key::Key2)
            || self.window.is_key_down(Key::Key3)
            || self.window.is_key_down(Key::Key4)
            || self.window.is_key_down(Key::X)
        {
            cycle_scene = true;
        }

        if toggle_view {
            self.view_mode = match self.view_mode {
                ViewMode::Blades => {
                    println!("  [UI MODE] Modo de Visualizacao Direta da Arvore XUI da Microsoft (.XUR) ativado!");
                    ViewMode::XuiDirect
                }
                ViewMode::XuiDirect => {
                    println!("  [UI MODE] Modo Dashboard Blades ativado!");
                    ViewMode::Blades
                }
            };
            self.key_cooldown = 15;
            return;
        }

        if cycle_scene && !self.xui_scenes.is_empty() {
            self.current_scene_idx = (self.current_scene_idx + 1) % self.xui_scenes.len();
            println!("  [XUI SCENE] Cena ativa alterada para: '{}'", self.xui_scenes[self.current_scene_idx].name);
            self.key_cooldown = 15;
            return;
        }

        let mut changed = false;

        // Poll physical USB Gamepad via XInput
        let mut pad_left = false;
        let mut pad_right = false;
        let mut pad_up = false;
        let mut pad_down = false;
        let mut pad_a = false;
        let mut pad_b = false;

        if let Some(state) = self.gamepad_mgr.poll(0) {
            let btns = state.gamepad.w_buttons;
            if (btns & XINPUT_GAMEPAD_DPAD_LEFT) != 0
                || (btns & XINPUT_GAMEPAD_LEFT_SHOULDER) != 0
                || state.gamepad.s_thumb_lx < -16000
            {
                pad_left = true;
            }
            if (btns & XINPUT_GAMEPAD_DPAD_RIGHT) != 0
                || (btns & XINPUT_GAMEPAD_RIGHT_SHOULDER) != 0
                || state.gamepad.s_thumb_lx > 16000
            {
                pad_right = true;
            }
            if (btns & XINPUT_GAMEPAD_DPAD_UP) != 0 || state.gamepad.s_thumb_ly > 16000 {
                pad_up = true;
            }
            if (btns & XINPUT_GAMEPAD_DPAD_DOWN) != 0 || state.gamepad.s_thumb_ly < -16000 {
                pad_down = true;
            }
            if (btns & XINPUT_GAMEPAD_A) != 0 {
                pad_a = true;
            }
            if (btns & XINPUT_GAMEPAD_B) != 0 {
                pad_b = true;
            }
        }

        // Left / Right (Switch Blades) - Keyboard Arrow Keys / A,D or Gamepad DPad / Bumpers / Stick
        if self.window.is_key_down(Key::Left) || self.window.is_key_down(Key::A) || pad_left {
            self.active_blade = if self.active_blade == 0 {
                self.blades.len() - 1
            } else {
                self.active_blade - 1
            };
            self.selected_item = 0;
            self.key_cooldown = 12; // 200ms debounce at 60 FPS
            changed = true;
        } else if self.window.is_key_down(Key::Right) || self.window.is_key_down(Key::D) || pad_right {
            self.active_blade = (self.active_blade + 1) % self.blades.len();
            self.selected_item = 0;
            self.key_cooldown = 12;
            changed = true;
        }

        // Up / Down (Switch Menu items) - Keyboard Arrow Keys / W,S or Gamepad DPad / Stick
        if self.window.is_key_down(Key::Up) || self.window.is_key_down(Key::W) || pad_up {
            let count = self.blades[self.active_blade].items.len();
            self.selected_item = if self.selected_item == 0 {
                count - 1
            } else {
                self.selected_item - 1
            };
            self.key_cooldown = 10;
            changed = true;
        } else if self.window.is_key_down(Key::Down) || self.window.is_key_down(Key::S) || pad_down {
            let count = self.blades[self.active_blade].items.len();
            self.selected_item = (self.selected_item + 1) % count;
            self.key_cooldown = 10;
            changed = true;
        }

        if pad_a {
            println!(
                "  [USB GAMEPAD] Button (A) pressed on item: '{}'",
                self.blades[self.active_blade].items[self.selected_item].title
            );
            self.key_cooldown = 14;
        }
        if pad_b {
            println!("  [USB GAMEPAD] Button (B) pressed");
            self.key_cooldown = 14;
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
        // If in startup sequence, render the Xbox 360 boot animation
        if self.mode == DisplayMode::BootAnimation {
            self.render_boot_animation();
            self.boot_frame += 1;
            if self.boot_frame >= 280 {
                println!("  [BOOTANIM] Sequencia de boot do Xbox 360 concluida -> Carregando Dashboard ($flash_dash.xex)...");
                self.mode = DisplayMode::Dashboard;
            }
            return;
        }

        // If in XuiDirect mode, render authentic Microsoft XUI scene graph directly from dump
        if self.view_mode == ViewMode::XuiDirect {
            self.render_xui_direct_scene();
            return;
        }

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
                &b.name,
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

            let (fill_top, fill_bottom, border_color) = if is_selected {
                (0xF020_6520, 0xF010_3E10, 0xFF00_FF7F)
            } else {
                (0xD018_2218, 0xD00C_140C, 0x803E_4E3E)
            };

            rasterize_rounded_card(
                &mut self.framebuffer,
                w,
                h,
                card_x as f32,
                cy as f32,
                card_w as f32,
                card_h as f32,
                10.0,
                fill_top,
                fill_bottom,
                border_color,
            );

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
                &item.title,
                title_color,
                2, // Big 16px title
            );

            draw_text(
                &mut self.framebuffer,
                w,
                h,
                card_x + 97,
                cy + 58,
                &item.subtitle,
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

        // Button (Y) / TAB: XUI Direct Scene
        draw_button_circle(&mut self.framebuffer, w, h, bx, by + 4, 0x00E0A000);
        draw_text(&mut self.framebuffer, w, h, bx - 3, by, "Y", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, bx + 16, by, "XUI .xur [TAB]", 0x0000FF7F, 1);
        bx += 160;

        // Navigation hints
        draw_text(
            &mut self.framebuffer,
            w,
            h,
            bx,
            by,
            "[<] [>] Alternar Blades (A/D ou Setas)",
            0x0090D0FF,
            1,
        );

        let pad_status = if self.gamepad_mgr.controller_connected {
            "[PAD 1: USB CONNECTED]"
        } else {
            "[KEYBOARD MODE]"
        };
        draw_text(
            &mut self.framebuffer,
            w,
            h,
            w - 580,
            by,
            pad_status,
            if self.gamepad_mgr.controller_connected { 0x0052D053 } else { 0x00FFB000 },
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

    /// Renders the iconic authentic Xbox 360 startup boot animation sequence
    pub fn render_boot_animation(&mut self) {
        let w = self.width;
        let h = self.height;
        let frame = self.boot_frame;

        // Deep pure void black background
        self.framebuffer.fill(0x00000000);

        let cx = (w / 2) as f32;
        let cy = (h / 2) as f32;

        // -----------------------------------------------------------
        // PHASE 1: Neon Green Aurora Spirals (Frames 0 to 145)
        // -----------------------------------------------------------
        if frame < 150 {
            let fade_ribbon = if frame < 110 {
                (frame as f32 / 30.0).min(1.0)
            } else {
                ((150 - frame) as f32 / 40.0).max(0.0)
            };

            let num_ribbons = 6;
            for r in 0..num_ribbons {
                let base_phase = r as f32 * (std::f32::consts::PI * 2.0 / num_ribbons as f32);
                let num_samples = 240;
                for s in 0..num_samples {
                    let progress = s as f32 / num_samples as f32;
                    let t = frame as f32 * 0.05 + base_phase;
                    let rad = (1.0 - progress) * (520.0 - (frame as f32 * 2.2).min(380.0)) + 40.0;
                    let angle = progress * 7.0 + t + (s as f32 * 0.015);

                    // Perspective projection: slightly flattened Y for 3D orbital depth
                    let px = (cx + rad * angle.cos()) as i32;
                    let py = (cy + rad * angle.sin() * 0.65) as i32;

                    let intensity = ((1.0 - progress) * fade_ribbon * 255.0) as u32;
                    if intensity == 0 {
                        continue;
                    }

                    // Additive luminous brush (3x3 with smooth distance falloff)
                    for dy in -2..=2 {
                        for dx in -2..=2 {
                            let qx = px + dx;
                            let qy = py + dy;
                            if qx >= 0 && qx < w as i32 && qy >= 0 && qy < h as i32 {
                                let idx = qy as usize * w + qx as usize;
                                let dist = ((dx * dx + dy * dy) as f32).sqrt();
                                let falloff = (1.0 - dist / 3.0).max(0.0);
                                let g_add = ((intensity as f32) * falloff) as u32;
                                let r_add = g_add / 3;
                                let b_add = g_add / 4;

                                let cur = self.framebuffer[idx];
                                let cur_r = (cur >> 16) & 0xFF;
                                let cur_g = (cur >> 8) & 0xFF;
                                let cur_b = cur & 0xFF;

                                let new_r = (cur_r + r_add).min(255);
                                let new_g = (cur_g + g_add).min(255);
                                let new_b = (cur_b + b_add).min(255);

                                self.framebuffer[idx] = (new_r << 16) | (new_g << 8) | new_b;
                            }
                        }
                    }
                }
            }
        }

        // -----------------------------------------------------------
        // PHASE 2: Acoustic Radial Shockwave Rings (Frames 75 to 220)
        // -----------------------------------------------------------
        if frame >= 75 && frame <= 220 {
            let num_waves = 3;
            for w_idx in 0..num_waves {
                let start_f = 75 + w_idx * 35;
                if frame >= start_f {
                    let wave_age = (frame - start_f) as f32;
                    let wave_rad = wave_age * 9.5;
                    let max_rad = 620.0f32;
                    if wave_rad < max_rad {
                        let alpha = (1.0 - wave_rad / max_rad).max(0.0);
                        let num_ring_pts = (wave_rad * 3.14).min(1000.0) as usize;
                        for i in 0..num_ring_pts {
                            let theta = (i as f32 / num_ring_pts as f32) * std::f32::consts::PI * 2.0;
                            let rx = (cx + wave_rad * theta.cos()) as i32;
                            let ry = (cy + wave_rad * theta.sin() * 0.70) as i32;
                            if rx >= 0 && rx < w as i32 && ry >= 0 && ry < h as i32 {
                                let idx = ry as usize * w + rx as usize;
                                let cur = self.framebuffer[idx];
                                let g_add = (alpha * 120.0) as u32;
                                let cur_g = (cur >> 8) & 0xFF;
                                let new_g = (cur_g + g_add).min(255);
                                self.framebuffer[idx] = (cur & 0xFFFF00FF) | (new_g << 8);
                            }
                        }
                    }
                }
            }
        }

        // -----------------------------------------------------------
        // PHASE 3: 3D Ray-Traced Chrome Sphere & Ring of Light (Frames 85 to 280)
        // -----------------------------------------------------------
        if frame >= 85 {
            // Target sphere center and radius:
            // Settles at (cx, cy) with radius 88, then in frames 240..275 slides to (42, 35) with radius 22
            let (scx, scy, sradius) = if frame < 240 {
                let r = if frame < 120 {
                    ((frame - 85) as f32 / 35.0) * 88.0
                } else {
                    88.0
                };
                (cx, cy, r)
            } else {
                let p = ((frame - 240) as f32 / 35.0).min(1.0);
                let smooth_p = p * p * (3.0 - 2.0 * p); // smooth ease-in-out curve
                let x = cx + (42.0 - cx) * smooth_p;
                let y = cy + (35.0 - cy) * smooth_p;
                let r = 88.0 + (22.0 - 88.0) * smooth_p;
                (x, y, r)
            };

            let bound = (sradius + 26.0) as i32;
            let min_y = ((scy - bound as f32) as i32).max(0) as usize;
            let max_y = ((scy + bound as f32) as i32).min(h as i32 - 1) as usize;
            let min_x = ((scx - bound as f32) as i32).max(0) as usize;
            let max_x = ((scx + bound as f32) as i32).min(w as i32 - 1) as usize;

            // RoL Quadrant Status:
            // 85..170: Spinning Chase
            // 170..185: Synchronous Flash of all 4 quads
            // 185+: Quadrant 0 (Top-Left, Player 1) locked solid green!
            let active_quad = if frame < 170 {
                ((frame - 85) / 8) % 4
            } else if frame <= 185 {
                99 // All 4 quads flash
            } else {
                0 // Player 1 solid green
            };

            for y in min_y..=max_y {
                let dy = y as f32 - scy;
                for x in min_x..=max_x {
                    let dx = x as f32 - scx;
                    let dist = (dx * dx + dy * dy).sqrt();

                    // Outer Ring of Light Bezel & 4 Quadrants
                    let rol_inner = sradius + 5.0;
                    let rol_outer = sradius + 19.0;
                    if dist >= rol_inner && dist <= rol_outer {
                        // Divider cross between quadrants
                        if dx.abs() < 2.5 || dy.abs() < 2.5 {
                            self.framebuffer[y * w + x] = 0x00141814;
                        } else {
                            let quad_idx = if dx < 0.0 && dy < 0.0 {
                                0 // Top-Left (Player 1)
                            } else if dx > 0.0 && dy < 0.0 {
                                1 // Top-Right
                            } else if dx > 0.0 && dy > 0.0 {
                                2 // Bottom-Right
                            } else {
                                3 // Bottom-Left
                            };

                            let is_lit = active_quad == 99 || active_quad == quad_idx;
                            if is_lit {
                                let pulse = ((frame as f32 * 0.2).sin() * 0.08 + 0.92) as f32;
                                let lr = (0x52 as f32 * pulse) as u32;
                                let lg = (0xD0 as f32 * pulse) as u32;
                                let lb = (0x53 as f32 * pulse) as u32;
                                self.framebuffer[y * w + x] = (lr << 16) | (lg << 8) | lb;
                            } else {
                                self.framebuffer[y * w + x] = 0x001B331B;
                            }
                        }
                    } else if dist >= rol_outer && dist <= rol_outer + 3.0 {
                        self.framebuffer[y * w + x] = 0x00505A50;
                    } else if dist < sradius {
                        // 3D Ray-Traced Chrome Sphere Shading
                        let nx = dx / sradius;
                        let ny = dy / sradius;
                        let nz = (1.0 - (nx * nx + ny * ny).min(1.0)).sqrt();

                        // Main light direction (-0.45, -0.55, 0.70)
                        let lx = -0.45f32;
                        let ly = -0.55f32;
                        let lz = 0.70f32;
                        let dot = (nx * lx + ny * ly + nz * lz).max(0.0);

                        // Specular Blinn-Phong highlight
                        let hx = -0.32f32;
                        let hy = -0.39f32;
                        let hz = 0.86f32;
                        let sdot = (nx * hx + ny * hy + nz * hz).max(0.0);
                        let spec = sdot.powf(20.0);

                        // Core glowing green energy inside the jewel
                        let core = (1.0 - dist / sradius).powf(1.8);

                        let r = ((25.0 * dot + 240.0 * spec + 15.0 * core) as u32).min(255);
                        let g = ((65.0 * dot + 255.0 * spec + 230.0 * core) as u32).min(255);
                        let b = ((30.0 * dot + 240.0 * spec + 25.0 * core) as u32).min(255);

                        self.framebuffer[y * w + x] = (r << 16) | (g << 8) | b;
                    }
                }
            }
        }

        // -----------------------------------------------------------
        // PHASE 4: Horizontal Lens Flare Burst (Frames 170 to 195)
        // -----------------------------------------------------------
        if frame >= 170 && frame <= 195 {
            let flare_progress = (frame - 170) as f32 / 25.0;
            let flare_alpha = (1.0 - flare_progress).max(0.0);
            let streak_h = 35;
            let sy_min = (cy as i32 - streak_h).max(0) as usize;
            let sy_max = (cy as i32 + streak_h).min(h as i32 - 1) as usize;

            for y in sy_min..=sy_max {
                let dy = (y as f32 - cy).abs();
                let v_falloff = (1.0 - dy / streak_h as f32).powf(2.0);
                for x in 0..w {
                    let dx = (x as f32 - cx).abs();
                    let h_falloff = (1.0 - dx / (w as f32 * 0.65)).max(0.0);
                    let intensity = (flare_alpha * v_falloff * h_falloff * 255.0) as u32;
                    if intensity > 0 {
                        let cur = self.framebuffer[y * w + x];
                        let r = ((cur >> 16) & 0xFF) + intensity;
                        let g = ((cur >> 8) & 0xFF) + intensity;
                        let b = (cur & 0xFF) + intensity;
                        self.framebuffer[y * w + x] = (r.min(255) << 16) | (g.min(255) << 8) | b.min(255);
                    }
                }
            }
        }

        // -----------------------------------------------------------
        // PHASE 5: Emblem "XBOX 360" & Subtitle (Frames 130 to 240)
        // -----------------------------------------------------------
        if frame >= 130 && frame < 245 {
            let text_alpha = if frame < 165 {
                (frame - 130) as f32 / 35.0
            } else if frame > 235 {
                (245 - frame) as f32 / 10.0
            } else {
                1.0
            };

            let brightness = (text_alpha * 255.0) as u32;
            let text_color = (brightness << 16) | (brightness << 8) | brightness;
            let shadow_color = (((brightness * 16) / 255) << 16) | (((brightness * 124) / 255) << 8) | ((brightness * 16) / 255);

            let logo_text = "XBOX 360";
            let logo_x = (w / 2) - 108;
            let logo_y = (cy as usize) + 120;

            // Glowing Green Shadow
            draw_text(&mut self.framebuffer, w, h, logo_x + 2, logo_y + 2, logo_text, shadow_color, 3);
            // Crisp White Chrome Logo
            draw_text(&mut self.framebuffer, w, h, logo_x, logo_y, logo_text, text_color, 3);

            let sub_text = "MICROSOFT CORPORATION";
            let sub_x = (w / 2) - 95;
            let sub_y = logo_y + 36;
            let sub_color = (((brightness * 90) / 255) << 16) | (((brightness * 140) / 255) << 8) | ((brightness * 90) / 255);
            draw_text(&mut self.framebuffer, w, h, sub_x, sub_y, sub_text, sub_color, 1);
        }

        // -----------------------------------------------------------
        // PHASE 6: Subtle Skip Hint
        // -----------------------------------------------------------
        let skip_prompt = "PRESS (A) OR SPACE TO SKIP";
        draw_text(&mut self.framebuffer, w, h, w - 240, h - 30, skip_prompt, 0x00406040, 1);
    }

    /// Injects authentic Microsoft XUI scenes (.xur) and localized string tables (.xus)
    pub fn apply_xui_locale(&mut self, xus: &crate::loader::XusTable, scenes: &[crate::loader::XurScene]) {
        self.xus_table = Some(xus.clone());
        self.xui_scenes = scenes.to_vec();
        println!("[+] GpuDisplay: Binding {} authentic XUI Scene(s) & Portuguese String Table (.xus)...", scenes.len());

        // Update Games Blade with genuine Portuguese strings directly from dump
        if let Some(s) = xus.find("Meus Jogos Arcade") {
            println!("  [XUI PT-BR] Binding Arcade title: '{}'", s);
            self.blades[1].items[2].title = "Xbox Live Arcade (Meus Jogos)".to_string();
            self.blades[1].items[2].subtitle = s.clone();
        }
        if let Some(s) = xus.find("Novo Jogo: Bejeweled") {
            println!("  [XUI PT-BR] Binding Spotlight item: '{}'", s);
            self.blades[1].items[3].title = s.clone();
            self.blades[1].items[3].subtitle = "Destaque Oficial da Microsoft no Dashboard".to_string();
        }
        if let Some(s) = xus.find("Aqui você obtém uma grande variedade") {
            println!("  [XUI PT-BR] Binding Arcade Description: '{}'", s);
            self.blades[1].items[0].subtitle = s.clone();
        }

        for sc in scenes {
            println!("  [XUI SCENE] Attached scene '{}' (Root: {}, Visual Nodes: {})",
                sc.name, sc.root.class_name, sc.root.children.len()
            );
        }
    }

    /// Renders the genuine Microsoft .xur (XUIB) scene graph directly using D3D9 barycentric cards & text
    pub fn render_xui_direct_scene(&mut self) {
        let w = self.width;
        let h = self.height;

        // Dark slate development / XUI canvas background with subtle grid
        for y in 0..h {
            for x in 0..w {
                let grid = (x % 32 == 0 || y % 32 == 0) as u32;
                let bg = if grid == 1 { 0x00141A22 } else { 0x000E1217 };
                self.framebuffer[y * w + x] = bg;
            }
        }

        // Top Header Bar
        for y in 0..65 {
            for x in 0..w {
                self.framebuffer[y * w + x] = 0x001A241A;
            }
        }
        for x in 0..w {
            self.framebuffer[64 * w + x] = 0x0052D053; // Xbox Emerald Green line
        }

        draw_text(&mut self.framebuffer, w, h, 25, 18, "MICROSOFT XUI ENGINE [ARVORE VISUAL .XUR REAL]", 0x0052D053, 2);
        draw_text(&mut self.framebuffer, w, h, 25, 42, "Direct3D 9 Barycentric Rasterizer | ATI Xenos EDRAM 1280x720 32-bit ARGB", 0x00A0B0A0, 1);

        let scene_count = self.xui_scenes.len();
        if scene_count == 0 {
            draw_text(&mut self.framebuffer, w, h, 400, 340, "Nenhuma cena XUI carregada.", 0x00FF8080, 2);
            return;
        }

        let scene = &self.xui_scenes[self.current_scene_idx % scene_count];

        // Scene Selector Tabs on Top Right
        let prompt = format!("[TAB / (Y)]: Voltar p/ Blades | [X / 1-{}]: Cena ({}/{})", scene_count, self.current_scene_idx + 1, scene_count);
        draw_text(&mut self.framebuffer, w, h, w - 580, 24, &prompt, 0x00FFFFFF, 1);

        // Scene Name Badge
        let badge_text = format!("ARQUIVO: {}.xur (Magic: XUIB v{})", scene.name, scene.version);
        draw_text(&mut self.framebuffer, w, h, w - 580, 42, &badge_text, 0x0000FF7F, 1);

        // Left Panel: Scene Info & Hierarchy Tree
        rasterize_rounded_card(
            &mut self.framebuffer,
            w,
            h,
            30.0,
            85.0,
            360.0,
            560.0,
            10.0,
            0xE01A221A,
            0xE00E140E,
            0xFF2A8C2A,
        );

        draw_text(&mut self.framebuffer, w, h, 50, 105, "METADADOS DA CENA (.XUR)", 0x0052D053, 2);
        let root_info = format!("Root Class: {}", scene.root.class_name);
        draw_text(&mut self.framebuffer, w, h, 50, 135, &root_info, 0x00FFFFFF, 1);
        let node_count = format!("Nos Visuais: {}", scene.root.children.len());
        draw_text(&mut self.framebuffer, w, h, 50, 155, &node_count, 0x00E0E0E0, 1);
        let class_count = format!("Classes XUI: {}", scene.classes.len());
        draw_text(&mut self.framebuffer, w, h, 50, 175, &class_count, 0x00E0E0E0, 1);

        draw_text(&mut self.framebuffer, w, h, 50, 210, "CLASSES REGISTRADAS:", 0x0000FF7F, 1);
        for (i, c) in scene.classes.iter().take(6).enumerate() {
            let line = format!("- {}", c);
            draw_text(&mut self.framebuffer, w, h, 60, 230 + i * 18, &line, 0x00A0C0A0, 1);
        }

        draw_text(&mut self.framebuffer, w, h, 50, 360, "STRINGS PT-BR (.XUS):", 0x0000FF7F, 1);
        if let Some(ref table) = self.xus_table {
            for (i, s) in table.strings.iter().take(8).enumerate() {
                let preview: String = s.chars().take(28).collect();
                let line = format!("\"{}\"", preview);
                draw_text(&mut self.framebuffer, w, h, 60, 385 + i * 20, &line, 0x00FFFFFF, 1);
            }
        }

        // Center / Main Canvas: Render the visual nodes extracted from the .xur
        rasterize_rounded_card(
            &mut self.framebuffer,
            w,
            h,
            415.0,
            85.0,
            835.0,
            560.0,
            12.0,
            0xE0102010,
            0xE0061006,
            0xFF00FF7F,
        );

        let canvas_title = format!("RENDERIZACAO DA ARVORE VISUAL: {}", scene.name);
        draw_text(&mut self.framebuffer, w, h, 440, 105, &canvas_title, 0x00FFFFFF, 2);

        // Render each child visual node from the genuine .xur
        let items = &scene.root.children;
        if items.is_empty() {
            draw_text(&mut self.framebuffer, w, h, 440, 160, "Container de cena raiz sem nós filhos diretos.", 0x00A0A0A0, 1);
        } else {
            for (idx, node) in items.iter().take(6).enumerate() {
                let ny = 145.0 + idx as f32 * 75.0;
                let nw = 785.0f32;
                let nh = 62.0f32;
                let nx = 440.0f32;

                let fill_top = if idx % 2 == 0 { 0xF0183018 } else { 0xF0102410 };
                let fill_bot = 0xF0081408;
                let border = 0xFF52D053;

                rasterize_rounded_card(
                    &mut self.framebuffer,
                    w,
                    h,
                    nx,
                    ny,
                    nw,
                    nh,
                    8.0,
                    fill_top,
                    fill_bot,
                    border,
                );

                // Control Tag & ID
                let node_title = format!("[#{:02}] {} - id=\"{}\"", idx + 1, node.class_name, node.id);
                draw_text(&mut self.framebuffer, w, h, (nx + 20.0) as usize, (ny + 12.0) as usize, &node_title, 0x00FFFFFF, 1);

                // Bound coordinates from VECT chunk
                let bounds_str = format!("XUI Bounds: ({:.0}, {:.0}, {:.0}, {:.0}) | D3D9 Viewport Active", node.x, node.y, node.width, node.height);
                draw_text(&mut self.framebuffer, w, h, (nx + 22.0) as usize, (ny + 34.0) as usize, &bounds_str, 0x00A0D0A0, 1);

                // Associate with real Portuguese string if available
                if let Some(ref table) = self.xus_table {
                    if idx < table.strings.len() {
                        let sample_s = &table.strings[idx];
                        let txt_preview: String = sample_s.chars().take(30).collect();
                        let mapped = format!("Texto: \"{}\"", txt_preview);
                        draw_text(&mut self.framebuffer, w, h, (nx + nw - 340.0) as usize, (ny + 22.0) as usize, &mapped, 0x0000FF7F, 1);
                    }
                }
            }
        }

        // Bottom Navigation Bar
        for y in (h - 55)..h {
            for x in 0..w {
                self.framebuffer[y * w + x] = 0x00141C14;
            }
        }
        for x in 0..w {
            self.framebuffer[(h - 55) * w + x] = 0x0052D053;
        }

        draw_text(&mut self.framebuffer, w, h, 40, h - 38, "[TAB] Voltar para Blades", 0x00FFFFFF, 1);
        draw_text(&mut self.framebuffer, w, h, 260, h - 38, "[X] / [1-4] Trocar Cena XUI", 0x0000FF7F, 1);
        draw_text(&mut self.framebuffer, w, h, 520, h - 38, "[ESC] Fechar Emulador", 0x00A0A0A0, 1);
        let status = format!("Xenos Ring Buffer: 0x10000000 | EDRAM Target: 0x70000000 (1280x720 60Hz)");
        draw_text(&mut self.framebuffer, w, h, w - 460, h - 38, &status, 0x0080C080, 1);
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
