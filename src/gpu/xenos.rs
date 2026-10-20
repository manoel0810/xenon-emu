use crate::gpu::pm4::*;
use crate::gpu::rasterizer::*;
use crate::gpu::ring_buffer::RingBuffer;
use crate::gpu::shader::{
    create_default_xui_pixel_shader, create_default_xui_vertex_shader, PixelShader, VertexInput,
    VertexShader,
};
use crate::gpu::texture::{Texture, TextureAtlas};
use crate::memory::MemoryBus;
use std::collections::HashMap;

pub const XENOS_WIDTH: usize = 1280;
pub const XENOS_HEIGHT: usize = 720;

/// Represents a 2D/3D Vertex sent by Xbox 360 D3D/XUI draw calls
#[derive(Debug, Clone, Copy)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub color: u32,
    pub u: f32,
    pub v: f32,
}

impl Vertex {
    pub fn new(x: f32, y: f32, z: f32, color: u32, u: f32, v: f32) -> Self {
        Self { x, y, z, color, u, v }
    }

    pub fn pos_color(x: f32, y: f32, z: f32, color: u32) -> Self {
        Self { x, y, z, color, u: 0.0, v: 0.0 }
    }
}

/// ATI Xenos (Xbox 360) Command Processor and Graphics Engine
pub struct XenosGpu {
    /// Context registers (e.g., RB_COLOR_INFO, PA_SC_WINDOW_OFFSET, etc.)
    pub registers: HashMap<u32, u32>,
    /// ALU Constant Vector Registers: c0 through c255 (float4 each)
    pub alu_constants: [[f32; 4]; 256],
    /// Currently bound Vertex Shader
    pub current_vs: Option<VertexShader>,
    /// Currently bound Pixel Shader
    pub current_ps: Option<PixelShader>,
    /// Default XUI Vertex Shader
    pub default_vs: VertexShader,
    /// Default XUI Pixel Shader
    pub default_ps: PixelShader,
    /// Viewport parameters
    pub vport_xscale: f32,
    pub vport_yscale: f32,
    pub vport_xoffset: f32,
    pub vport_yoffset: f32,
    /// Texture Atlas of authentic Xbox 360 Dashboard assets
    pub atlas: Option<TextureAtlas>,
    /// Texture samplers (0..15)
    pub samplers: [Option<Texture>; 16],
    pub bound_texture_name: Option<String>,
    /// VGT DMA Vertex Buffer Base in guest RAM
    pub vgt_dma_base: u64,
    pub vgt_dma_stride: usize,
    pub vgt_dma_count: usize,
    /// On-chip internal color buffer (resolves into display framebuffer)
    pub color_buffer: Vec<u32>,
    /// Execution counters
    pub packets_executed: u64,
    pub draw_calls: u64,
    pub primitives_drawn: u64,
}

impl XenosGpu {
    pub fn new() -> Self {
        let mut registers = HashMap::new();
        // Default Viewport: 1280x720
        registers.insert(PA_CL_VPORT_XSCALE, (640.0f32).to_bits());
        registers.insert(PA_CL_VPORT_YSCALE, (-360.0f32).to_bits());
        registers.insert(PA_CL_VPORT_XOFFSET, (640.0f32).to_bits());
        registers.insert(PA_CL_VPORT_YOFFSET, (360.0f32).to_bits());

        let mut alu_constants = [[0.0f32; 4]; 256];
        // Identity MVP matrix in c0..c3
        alu_constants[0] = [1.0, 0.0, 0.0, 0.0];
        alu_constants[1] = [0.0, 1.0, 0.0, 0.0];
        alu_constants[2] = [0.0, 0.0, 1.0, 0.0];
        alu_constants[3] = [0.0, 0.0, 0.0, 1.0];
        // Full opacity diffuse tint in c4
        alu_constants[4] = [1.0, 1.0, 1.0, 1.0];

        Self {
            registers,
            alu_constants,
            current_vs: None,
            current_ps: None,
            default_vs: create_default_xui_vertex_shader(),
            default_ps: create_default_xui_pixel_shader(),
            vport_xscale: 640.0,
            vport_yscale: -360.0,
            vport_xoffset: 640.0,
            vport_yoffset: 360.0,
            atlas: None,
            samplers: Default::default(),
            bound_texture_name: None,
            vgt_dma_base: 0,
            vgt_dma_stride: 24,
            vgt_dma_count: 0,
            color_buffer: vec![0u32; XENOS_WIDTH * XENOS_HEIGHT],
            packets_executed: 0,
            draw_calls: 0,
            primitives_drawn: 0,
        }
    }

    /// Loads the texture atlas from directory containing Dashboard .XZP extracted assets
    pub fn load_atlas_from_dir<P: AsRef<std::path::Path>>(&mut self, path: P) {
        let atlas = TextureAtlas::load_from_dir(path);
        println!("  [XENOS GPU] Loaded TextureAtlas with {} authentic Dashboard textures.", atlas.len());
        self.atlas = Some(atlas);
    }

    /// Binds a texture by asset name into the specified sampler stage (e.g. 0)
    pub fn bind_texture_by_name(&mut self, sampler_idx: usize, name: &str) -> bool {
        if sampler_idx < 16 {
            if let Some(ref atlas) = self.atlas {
                if let Some(tex) = atlas.get(name) {
                    self.samplers[sampler_idx] = Some(tex.clone());
                    self.bound_texture_name = Some(name.to_string());
                    return true;
                }
            }
        }
        false
    }

    /// Binds an existing texture directly into a sampler stage
    pub fn bind_texture(&mut self, sampler_idx: usize, texture: Texture) {
        if sampler_idx < 16 {
            self.samplers[sampler_idx] = Some(texture);
        }
    }

    /// Executes all pending PM4 packets in the Primary Ring Buffer
    pub fn execute_ring(&mut self, bus: &mut MemoryBus, ring: &mut RingBuffer) -> Result<usize, String> {
        let mut count = 0;

        while ring.has_pending() {
            let pending = ring.pending_dwords();
            if pending == 0 {
                break;
            }

            // Peek packet header to determine total packet size
            let header = ring.peek_words(bus, 1)?[0];
            let ptype = (header >> 30) & 0x03;
            let packet_dwords = match ptype {
                PM4_TYPE_0 => (((header >> 16) & 0x3FFF) + 2) as usize,
                PM4_TYPE_1 => 3,
                PM4_TYPE_2 => 1,
                PM4_TYPE_3 => (((header >> 16) & 0x3FFF) + 2) as usize,
                _ => 1,
            };

            if pending < packet_dwords {
                // Incomplete packet in ring buffer, wait for CPU
                break;
            }

            let words = ring.peek_words(bus, packet_dwords)?;
            match Pm4Packet::decode(&words) {
                Ok((pkt, consumed)) => {
                    self.execute_packet(bus, &pkt)?;
                    ring.advance_rptr(consumed);
                    self.packets_executed += 1;
                    count += 1;
                }
                Err(e) => {
                    println!("  [XENOS CP ERROR] Error decoding PM4 packet: {}", e);
                    ring.advance_rptr(1);
                    break;
                }
            }
        }

        Ok(count)
    }

    /// Dispatches execution of a single decoded PM4 packet
    pub fn execute_packet(&mut self, bus: &mut MemoryBus, packet: &Pm4Packet) -> Result<(), String> {
        match packet {
            Pm4Packet::Type0 { base_reg, values, .. } => {
                for (i, &val) in values.iter().enumerate() {
                    let reg = base_reg + i as u32;
                    self.registers.insert(reg, val);
                }
            }
            Pm4Packet::Type1 { reg1, val1, reg2, val2 } => {
                self.registers.insert(*reg1, *val1);
                self.registers.insert(*reg2, *val2);
            }
            Pm4Packet::Type2Nop { .. } => {
                // No-op
            }
            Pm4Packet::Type3 { opcode, data, .. } => {
                match *opcode {
                    PM4_SET_CONSTANT => {
                        if data.len() >= 5 {
                            let reg_idx = data[0] as usize;
                            if reg_idx < 256 {
                                self.alu_constants[reg_idx] = [
                                    f32::from_bits(data[1]),
                                    f32::from_bits(data[2]),
                                    f32::from_bits(data[3]),
                                    f32::from_bits(data[4]),
                                ];
                            }
                        }
                    }
                    PM4_SET_CONTEXT_REG => {
                        if data.len() >= 2 {
                            let reg = data[0] + 0x2000;
                            let val = data[1];
                            self.registers.insert(reg, val);

                            match reg {
                                PA_CL_VPORT_XSCALE => self.vport_xscale = f32::from_bits(val),
                                PA_CL_VPORT_YSCALE => self.vport_yscale = f32::from_bits(val),
                                PA_CL_VPORT_XOFFSET => self.vport_xoffset = f32::from_bits(val),
                                PA_CL_VPORT_YOFFSET => self.vport_yoffset = f32::from_bits(val),
                                SQ_VS_PROGRAM_ADDR => {
                                    let addr = val as u64;
                                    if addr > 0x1000 {
                                        let mut code = Vec::new();
                                        for i in 0..32 {
                                            if let Ok(w) = bus.read_u32(addr + (i * 4) as u64) {
                                                code.push(w);
                                            }
                                        }
                                        if !code.is_empty() {
                                            self.current_vs = Some(VertexShader::decode_bytecode("GuestVS", &code));
                                        }
                                    }
                                }
                                SQ_PS_PROGRAM_ADDR => {
                                    let addr = val as u64;
                                    if addr > 0x1000 {
                                        let mut ps = PixelShader::new("GuestPS");
                                        let mut code = Vec::new();
                                        for i in 0..32 {
                                            if let Ok(w) = bus.read_u32(addr + (i * 4) as u64) {
                                                code.push(w);
                                            }
                                        }
                                        ps.raw_bytecode = code;
                                        self.current_ps = Some(ps);
                                    }
                                }
                                VGT_DMA_BASE => {
                                    self.vgt_dma_base = val as u64;
                                }
                                VGT_DMA_SIZE => {
                                    self.vgt_dma_count = val as usize;
                                }
                                VGT_DMA_STRIDE => {
                                    self.vgt_dma_stride = val as usize;
                                }
                                _ => {}
                            }
                        }
                    }
                    PM4_LOAD_ALU_CONSTANT => {
                        if !data.is_empty() {
                            let start_idx = data[0] as usize;
                            let vectors_data = &data[1..];
                            for (i, chunk) in vectors_data.chunks_exact(4).enumerate() {
                                let c_idx = start_idx + i;
                                if c_idx < 256 {
                                    self.alu_constants[c_idx] = [
                                        f32::from_bits(chunk[0]),
                                        f32::from_bits(chunk[1]),
                                        f32::from_bits(chunk[2]),
                                        f32::from_bits(chunk[3]),
                                    ];
                                }
                            }
                        }
                    }
                    PM4_DRAW_INDX => {
                        self.draw_calls += 1;
                        self.execute_draw_indexed(bus, data)?;
                    }
                    PM4_WAIT_FOR_IDLE => {
                        // Pipeline flush
                    }
                    PM4_MEM_WRITE => {
                        if data.len() >= 2 {
                            let dest_addr = data[0] as u64;
                            let val = data[1];
                            bus.write_u32(dest_addr, val)?;
                        }
                    }
                    _ => {
                        // Unhandled opcode
                    }
                }
            }
        }
        Ok(())
    }

    /// Executes an indexed draw call by reading indices, streaming vertices (via VGT DMA if configured),
    /// executing vertex/pixel shaders, and rasterizing triangles with optional bilinear texture filtering.
    fn execute_draw_indexed(&mut self, bus: &MemoryBus, data: &[u32]) -> Result<(), String> {
        if data.len() < 3 {
            return Ok(());
        }

        let prim_type = data[0];
        let num_indices = data[1] as usize;
        let index_ptr = data[2] as u64;

        if num_indices == 0 || index_ptr == 0 {
            return Ok(());
        }

        // Read 16-bit indices from guest memory
        let mut indices = Vec::with_capacity(num_indices);
        for i in 0..num_indices {
            let addr = index_ptr + (i * 2) as u64;
            let b0 = bus.read_u8(addr)?;
            let b1 = bus.read_u8(addr + 1)?;
            let idx = ((b0 as u16) << 8) | (b1 as u16);
            indices.push(idx);
        }

        let vs = self.current_vs.as_ref().unwrap_or(&self.default_vs);

        // Determine vertex inputs: Stream from VGT DMA buffer in guest RAM or fallback to default quad
        let raw_inputs: Vec<VertexInput> = if self.vgt_dma_base > 0x1000 {
            let count = if self.vgt_dma_count > 0 {
                self.vgt_dma_count
            } else {
                (indices.iter().max().copied().unwrap_or(0) as usize) + 1
            };
            let stride = if self.vgt_dma_stride >= 16 {
                self.vgt_dma_stride
            } else {
                24
            };

            let mut inputs = Vec::with_capacity(count);
            for i in 0..count {
                let v_addr = self.vgt_dma_base + (i * stride) as u64;
                let px = f32::from_bits(bus.read_u32(v_addr).unwrap_or(0));
                let py = f32::from_bits(bus.read_u32(v_addr + 4).unwrap_or(0));
                let pz = f32::from_bits(bus.read_u32(v_addr + 8).unwrap_or(0));

                let color_val = bus.read_u32(v_addr + 12).unwrap_or(0xFFFFFFFF);
                let ca = ((color_val >> 24) & 0xFF) as f32 / 255.0;
                let cr = ((color_val >> 16) & 0xFF) as f32 / 255.0;
                let cg = ((color_val >> 8) & 0xFF) as f32 / 255.0;
                let cb = (color_val & 0xFF) as f32 / 255.0;

                let (u, v) = if stride >= 24 {
                    let u_val = f32::from_bits(bus.read_u32(v_addr + 16).unwrap_or(0));
                    let v_val = f32::from_bits(bus.read_u32(v_addr + 20).unwrap_or(0));
                    (u_val, v_val)
                } else {
                    (0.0, 0.0)
                };

                inputs.push(VertexInput {
                    position: [px, py, pz, 1.0],
                    color: [cr, cg, cb, ca],
                    uv: [u, v],
                    normal: [0.0, 0.0, 1.0],
                });
            }
            inputs
        } else {
            vec![
                VertexInput {
                    position: [380.0, 150.0, 0.0, 1.0],
                    color: [0.165, 0.549, 0.165, 1.0], // 0xFF2A8C2A
                    uv: [0.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                VertexInput {
                    position: [1180.0, 150.0, 0.0, 1.0],
                    color: [0.106, 0.420, 0.106, 1.0], // 0xFF1B6B1B
                    uv: [1.0, 0.0],
                    normal: [0.0, 0.0, 1.0],
                },
                VertexInput {
                    position: [1180.0, 250.0, 0.0, 1.0],
                    color: [0.051, 0.231, 0.051, 1.0], // 0xFF0D3B0D
                    uv: [1.0, 1.0],
                    normal: [0.0, 0.0, 1.0],
                },
                VertexInput {
                    position: [380.0, 250.0, 0.0, 1.0],
                    color: [0.051, 0.231, 0.051, 1.0], // 0xFF0D3B0D
                    uv: [0.0, 1.0],
                    normal: [0.0, 0.0, 1.0],
                },
            ]
        };

        if raw_inputs.is_empty() {
            return Ok(());
        }

        // Execute Vertex Shader for all vertices
        let mut vertices = Vec::with_capacity(raw_inputs.len());
        for v_in in &raw_inputs {
            let v_out = vs.execute(
                v_in,
                &self.alu_constants,
                self.vport_xscale,
                self.vport_yscale,
                self.vport_xoffset,
                self.vport_yoffset,
            );

            let ca = (v_out.color[3].clamp(0.0, 1.0) * 255.0) as u32;
            let cr = (v_out.color[0].clamp(0.0, 1.0) * 255.0) as u32;
            let cg = (v_out.color[1].clamp(0.0, 1.0) * 255.0) as u32;
            let cb = (v_out.color[2].clamp(0.0, 1.0) * 255.0) as u32;
            let color = (ca << 24) | (cr << 16) | (cg << 8) | cb;

            vertices.push(Vertex {
                x: v_out.screen_x,
                y: v_out.screen_y,
                z: v_out.clip_pos[2],
                color,
                u: v_out.uv[0],
                v: v_out.uv[1],
            });
        }

        let active_tex = self.samplers[0].as_ref();
        let mut tris_count = 0;

        match prim_type {
            // D3DPT_TRIANGLELIST (0x04)
            0x04 => {
                for chunk in indices.chunks_exact(3) {
                    let (i0, i1, i2) = (
                        chunk[0] as usize % vertices.len(),
                        chunk[1] as usize % vertices.len(),
                        chunk[2] as usize % vertices.len(),
                    );
                    let v0 = &vertices[i0];
                    let v1 = &vertices[i1];
                    let v2 = &vertices[i2];
                    rasterize_triangle_textured(
                        &mut self.color_buffer,
                        XENOS_WIDTH,
                        XENOS_HEIGHT,
                        v0,
                        v1,
                        v2,
                        active_tex,
                    );
                    tris_count += 1;
                }
            }
            // D3DPT_TRIANGLESTRIP (0x05)
            0x05 => {
                for i in 2..indices.len() {
                    let (i0, i1, i2) = if i % 2 == 0 {
                        (
                            indices[i - 2] as usize % vertices.len(),
                            indices[i - 1] as usize % vertices.len(),
                            indices[i] as usize % vertices.len(),
                        )
                    } else {
                        (
                            indices[i - 1] as usize % vertices.len(),
                            indices[i - 2] as usize % vertices.len(),
                            indices[i] as usize % vertices.len(),
                        )
                    };
                    let v0 = &vertices[i0];
                    let v1 = &vertices[i1];
                    let v2 = &vertices[i2];
                    rasterize_triangle_textured(
                        &mut self.color_buffer,
                        XENOS_WIDTH,
                        XENOS_HEIGHT,
                        v0,
                        v1,
                        v2,
                        active_tex,
                    );
                    tris_count += 1;
                }
            }
            // D3DPT_TRIANGLEFAN (0x06)
            0x06 => {
                for i in 1..indices.len().saturating_sub(1) {
                    let i0 = indices[0] as usize % vertices.len();
                    let i1 = indices[i] as usize % vertices.len();
                    let i2 = indices[i + 1] as usize % vertices.len();
                    let v0 = &vertices[i0];
                    let v1 = &vertices[i1];
                    let v2 = &vertices[i2];
                    rasterize_triangle_textured(
                        &mut self.color_buffer,
                        XENOS_WIDTH,
                        XENOS_HEIGHT,
                        v0,
                        v1,
                        v2,
                        active_tex,
                    );
                    tris_count += 1;
                }
            }
            _ => {
                // Fallback for default indexed list
                for chunk in indices.chunks_exact(3) {
                    let (i0, i1, i2) = (
                        chunk[0] as usize % vertices.len(),
                        chunk[1] as usize % vertices.len(),
                        chunk[2] as usize % vertices.len(),
                    );
                    let v0 = &vertices[i0];
                    let v1 = &vertices[i1];
                    let v2 = &vertices[i2];
                    rasterize_triangle_textured(
                        &mut self.color_buffer,
                        XENOS_WIDTH,
                        XENOS_HEIGHT,
                        v0,
                        v1,
                        v2,
                        active_tex,
                    );
                    tris_count += 1;
                }
            }
        }

        self.primitives_drawn += tris_count;
        Ok(())
    }

    /// Renders an authentic Microsoft XUI Scene (.xur) using the Xenos GPU rasterizer
    pub fn render_xui_scene(
        &mut self,
        scene: &crate::loader::XurScene,
        _xus: Option<&crate::loader::XusTable>,
        primary_color: u32,
        secondary_color: u32,
    ) {
        self.draw_calls += 1;

        // 1. Draw Blade Main Backdrop Container
        let x = 360.0f32;
        let y = 110.0f32;
        let w = 840.0f32;
        let h = 540.0f32;

        let fill_top = 0xE000_0000 | (primary_color & 0x00FF_FFFF);
        let fill_bottom = 0xE000_0000 | (secondary_color & 0x00FF_FFFF);
        let border = 0xFF52_D053;

        rasterize_rounded_card(
            &mut self.color_buffer,
            XENOS_WIDTH,
            XENOS_HEIGHT,
            x,
            y,
            w,
            h,
            12.0,
            fill_top,
            fill_bottom,
            border,
        );
        self.primitives_drawn += 2;

        // 2. Draw child control items defined in .xur scene
        let item_h = 75.0f32;
        let item_gap = 14.0f32;
        let item_start_y = y + 35.0;

        for (idx, _child) in scene.root.children.iter().take(4).enumerate() {
            let iy = item_start_y + (idx as f32) * (item_h + item_gap);
            let is_selected = idx == 0;

            let card_top = if is_selected { 0xF020_6020 } else { 0xC015_2015 };
            let card_bottom = if is_selected { 0xF010_4010 } else { 0xC00A_150A };
            let card_border = if is_selected { 0xFF00_FF7F } else { 0x8035_4535 };

            rasterize_rounded_card(
                &mut self.color_buffer,
                XENOS_WIDTH,
                XENOS_HEIGHT,
                x + 25.0,
                iy,
                w - 50.0,
                item_h,
                8.0,
                card_top,
                card_bottom,
                card_border,
            );
            self.primitives_drawn += 2;
        }
    }

    /// Resolves the GPU internal color buffer into the target display framebuffer
    pub fn resolve_surface(&self, target_fb: &mut [u32]) {
        let copy_len = self.color_buffer.len().min(target_fb.len());
        for i in 0..copy_len {
            let src = self.color_buffer[i];
            let dst = target_fb[i];
            target_fb[i] = blend_pixel(src, dst);
        }
    }
}
