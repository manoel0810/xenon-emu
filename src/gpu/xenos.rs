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

/// Decoded ATI Xenos Texture Resource Descriptor (SQ_TEX_RESOURCE_WORD0..5)
#[derive(Debug, Clone, Default)]
pub struct XenosTextureResource {
    pub words: [u32; 6],
}

impl XenosTextureResource {
    pub fn is_valid(&self) -> bool {
        self.base_address() != 0 && self.width() > 0 && self.height() > 0
    }

    /// Extracts physical memory base address from the 6-dword descriptor
    pub fn base_address(&self) -> u64 {
        let raw = if self.words[2] != 0 && (self.words[2] & 0x1000_0000) != 0 {
            self.words[2]
        } else if (self.words[1] & 0x1000_0000) != 0 || (self.words[1] & 0x9000_0000) != 0 {
            self.words[1]
        } else if self.words[0] != 0 && (self.words[0] & 0x1000_0000) != 0 {
            self.words[0]
        } else {
            self.words[1] & 0x7FFF_FFFF
        };
        (raw & 0x1FFF_FFFF) as u64
    }

    /// Extracts texture width from descriptor (words[2] bits [12:0] + 1)
    pub fn width(&self) -> usize {
        let w = (self.words[2] & 0x1FFF) as usize + 1;
        if w > 4096 { 1280 } else { w }
    }

    /// Extracts texture height from descriptor (words[2] bits [25:13] + 1)
    pub fn height(&self) -> usize {
        let h = ((self.words[2] >> 13) & 0x1FFF) as usize + 1;
        if h > 4096 { 720 } else { h }
    }

    /// Returns true if the texture format specifies 2D micro/macro tiling
    pub fn is_tiled(&self) -> bool {
        (self.words[3] & 0x8000_0000) != 0 || (self.words[0] & 0x0001_0000) != 0
    }
}

/// Decoded ATI Xenos Texture Sampler Descriptor (SQ_TEX_SAMPLER_WORD0..7)
#[derive(Debug, Clone, Copy, Default)]
pub struct XenosTextureSampler {
    pub raw: u32,
}

impl XenosTextureSampler {
    pub fn is_bilinear(&self) -> bool {
        ((self.raw >> 8) & 0x03) != 0 || ((self.raw >> 10) & 0x03) != 0
    }

    pub fn clamp_x(&self) -> u32 {
        self.raw & 0x07
    }

    pub fn clamp_y(&self) -> u32 {
        (self.raw >> 3) & 0x07
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
    /// Decoded Texture Resource Descriptors (SQ_TEX_RESOURCE_WORD0..5) for stages 0..15
    pub tex_resources: [XenosTextureResource; 16],
    /// Decoded Texture Sampler Descriptors (SQ_TEX_SAMPLER_WORD0..7) for stages 0..15
    pub tex_samplers: [XenosTextureSampler; 16],
    /// VGT DMA Vertex Buffer Base in guest RAM
    pub vgt_dma_base: u64,
    pub vgt_dma_stride: usize,
    pub vgt_dma_count: usize,
    /// On-chip internal color buffer (resolves into display framebuffer)
    pub color_buffer: Vec<u32>,
    /// Execution counters
    pub packets_executed: u64,
    /// Real Xenos opcodes (as emitted by the Direct3D driver in the dash) seen so far, by opcode value
    pub real_opcode_counts: HashMap<u8, u64>,
    /// GPU "counter" written by EVENT_WRITE_SHD when bit 31 of the initiator is set (increments per swap/event)
    pub gpu_counter: u32,
    /// Draws submitted with the real DRAW_INDX_2 / DRAW_INDX opcodes (not rasterised yet: the dash shaders are Xenos microcode)
    pub real_draws: u64,
    pub pending_interrupts: u64,
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
            tex_resources: Default::default(),
            tex_samplers: Default::default(),
            vgt_dma_base: 0,
            vgt_dma_stride: 24,
            vgt_dma_count: 0,
            color_buffer: vec![0u32; XENOS_WIDTH * XENOS_HEIGHT],
            packets_executed: 0,
            real_opcode_counts: HashMap::new(),
            gpu_counter: 0,
            real_draws: 0,
            pending_interrupts: 0,
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

    /// Executes pending PM4 packets and writes back the current RPTR to guest memory
    pub fn execute_ring_with_writeback(
        &mut self,
        bus: &mut MemoryBus,
        ring: &mut RingBuffer,
        writeback_addr: u64,
    ) -> Result<usize, String> {
        let count = self.execute_ring(bus, ring)?;
        if writeback_addr != 0 {
            let _ = bus.write_u32(writeback_addr, ring.rptr as u32);
        }
        Ok(count)
    }

    /// Dispatches a register write, updating state and decoding texture/sampler descriptors
    pub fn write_register(&mut self, reg: u32, val: u32, bus: &MemoryBus) {
        self.registers.insert(reg, val);

        // Texture Resource Descriptor: 0x0A00..0x0A5F (6 DWORDs per sampler stage 0..15)
        if (SQ_TEX_RESOURCE_BASE..SQ_TEX_RESOURCE_END).contains(&reg) {
            let offset = (reg - SQ_TEX_RESOURCE_BASE) as usize;
            let sampler_idx = offset / 6;
            let word_idx = offset % 6;
            if sampler_idx < 16 && word_idx < 6 {
                self.tex_resources[sampler_idx].words[word_idx] = val;
                self.update_sampler_from_resource(sampler_idx, bus);
            }
        }

        // Texture Sampler Descriptor: 0x0D00..0x0D0F
        if (SQ_TEX_SAMPLER_BASE..SQ_TEX_SAMPLER_END).contains(&reg) {
            let sampler_idx = (reg - SQ_TEX_SAMPLER_BASE) as usize;
            if sampler_idx < 16 {
                self.tex_samplers[sampler_idx] = XenosTextureSampler { raw: val };
            }
        }

        // Viewport and Geometry Context Registers
        match reg {
            PA_CL_VPORT_XSCALE => self.vport_xscale = f32::from_bits(val),
            PA_CL_VPORT_YSCALE => self.vport_yscale = f32::from_bits(val),
            PA_CL_VPORT_XOFFSET => self.vport_xoffset = f32::from_bits(val),
            PA_CL_VPORT_YOFFSET => self.vport_yoffset = f32::from_bits(val),
            VGT_DMA_BASE => self.vgt_dma_base = val as u64,
            VGT_DMA_SIZE => self.vgt_dma_count = val as usize,
            VGT_DMA_STRIDE => self.vgt_dma_stride = val as usize,
            _ => {}
        }
    }

    /// Dynamically constructs or updates a sampler stage from decoded hardware descriptors
    pub fn update_sampler_from_resource(&mut self, sampler_idx: usize, bus: &MemoryBus) {
        let res = &self.tex_resources[sampler_idx];
        let base_addr = res.base_address();
        let w = res.width();
        let h = res.height();

        if base_addr > 0 && w > 0 && h > 0 && w <= 4096 && h <= 4096 {
            let pixel_count = w * h;
            let byte_count = pixel_count * 4;
            if byte_count <= 16 * 1024 * 1024 {
                let mut pixels = Vec::with_capacity(pixel_count);
                let mut non_zero = 0;
                for i in 0..pixel_count {
                    let p_addr = base_addr + (i * 4) as u64;
                    if let Ok(pix) = bus.read_u32(p_addr) {
                        pixels.push(pix);
                        if pix != 0 {
                            non_zero += 1;
                        }
                    } else {
                        break;
                    }
                }
                if non_zero > 0 && pixels.len() == pixel_count {
                    self.samplers[sampler_idx] = Some(Texture::new(w, h, pixels));
                    println!("  [XENOS GPU] Sampler {}: Decoded dynamic texture ({}x{}, Base: 0x{:08X}, non-zero: {})",
                        sampler_idx, w, h, base_addr, non_zero
                    );
                    return;
                }
            }
        }

        // Fallback to authentic Dashboard asset if not yet populated in guest memory
        if self.samplers[sampler_idx].is_none() {
            if let Some(ref atlas) = self.atlas {
                let default_name = match sampler_idx {
                    0 => "xboxLogo",
                    1 => "Pip_bg",
                    2 => "guidebug_hd",
                    3 => "aura_floor",
                    _ => "xboxLogo",
                };
                if let Some(tex) = atlas.get(default_name) {
                    self.samplers[sampler_idx] = Some(tex.clone());
                }
            }
        }
    }

    /// Dispatches execution of a single decoded PM4 packet
    pub fn execute_packet(&mut self, bus: &mut MemoryBus, packet: &Pm4Packet) -> Result<(), String> {
        if pm4_log_enabled() {
            match packet {
                Pm4Packet::Type0 { base_reg, values, .. } => println!("  [PM4] T0 base=0x{:04X} n={} v0=0x{:08X}", base_reg, values.len(), values.first().copied().unwrap_or(0)),
                Pm4Packet::Type1 { reg1, val1, reg2, val2 } => println!("  [PM4] T1 0x{:04X}=0x{:08X} 0x{:04X}=0x{:08X}", reg1, val1, reg2, val2),
                Pm4Packet::Type2Nop { .. } => {}
                Pm4Packet::Type3 { opcode, data, .. } => {
                    let shown: Vec<String> = data.iter().take(8).map(|d| format!("{:08X}", d)).collect();
                    println!("  [PM4] T3 op=0x{:02X} ({}) n={} data=[{}]", opcode, Pm4Packet::opcode_name(*opcode), data.len(), shown.join(" "));
                }
            }
        }
        match packet {
            Pm4Packet::Type0 { base_reg, values, .. } => {
                for (i, &val) in values.iter().enumerate() {
                    let reg = base_reg + i as u32;
                    self.write_register(reg, val, bus);
                }
            }
            Pm4Packet::Type1 { reg1, val1, reg2, val2 } => {
                self.write_register(*reg1, *val1, bus);
                self.write_register(*reg2, *val2, bus);
            }
            Pm4Packet::Type2Nop { .. } => {
                // No-op
            }
            Pm4Packet::Type3 { opcode, data, .. } => {
                *self.real_opcode_counts.entry(*opcode).or_insert(0) += 1;
                match *opcode {
                    PM4_SET_CONSTANT if data.len() == 3 => {
                        // Real opcode 0x21 = REG_RMW: reg = data[0] & 0x1FFF, new = (old & and_mask) | or_mask
                        let reg = data[0] & 0x1FFF;
                        let old = *self.registers.get(&reg).unwrap_or(&0);
                        let and_mask = data[1];
                        let or_mask = data[2];
                        self.write_register(reg, (old & and_mask) | or_mask, bus);
                    }
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
                            self.write_register(reg, val, bus);

                            match reg {
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
                    PM4_DRAW_INDX | PM4_DRAW_INDX_2 => {
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
                    PM4_WAIT_REG_MEM => {
                        // Real layout (xenia command_processor.cc): [wait_info, poll_reg_or_addr, reference, mask, wait_interval]
                        if data.len() >= 4 {
                            let wait_info = data[0];
                            let poll_addr = data[1];
                            let reference = data[2];
                            let mask = data[3];

                            let is_mem = (wait_info & 0x10) != 0;
                            let function = wait_info & 0x07;
                            let mem_addr = (poll_addr & !0x3) as u64;

                            let current_val = if is_mem {
                                bus.read_u32(mem_addr).unwrap_or(0)
                            } else {
                                *self.registers.get(&poll_addr).unwrap_or(&0)
                            };

                            let masked = current_val & mask;
                            let satisfied = match function {
                                0 => false,        // never
                                1 => masked < reference,
                                2 => masked <= reference,
                                3 => masked == reference,
                                4 => masked != reference,
                                5 => masked >= reference,
                                6 => masked > reference,
                                _ => true,         // always
                            };

                            // The emulated GPU has no asynchronous hardware to make the condition true later, so a wait that
                            // is not yet satisfied is forced to completion instead of blocking the Command Processor.
                            if !satisfied {
                                let forced = (current_val & !mask) | (reference & mask);
                                if is_mem {
                                    let _ = bus.write_u32(mem_addr, forced);
                                } else {
                                    self.registers.insert(poll_addr, forced);
                                }
                            }
                        }
                    }                    PM4_ME_INIT => {
                        // Micro-engine initialization
                    }
                    PM4_EXECUTE_INDIRECT => {
                        if data.len() >= 2 {
                            let ib_addr = data[0] as u64;
                            let ib_count = data[1] as usize;
                            if ib_addr != 0 && ib_count > 0 {
                                let mut words = Vec::with_capacity(ib_count);
                                for i in 0..ib_count {
                                    if let Ok(w) = bus.read_u32(ib_addr + (i * 4) as u64) {
                                        words.push(w);
                                    }
                                }
                                let mut cur = 0;
                                while cur < words.len() {
                                    match Pm4Packet::decode(&words[cur..]) {
                                        Ok((sub_pkt, consumed)) => {
                                            self.execute_packet(bus, &sub_pkt)?;
                                            self.packets_executed += 1;
                                            cur += consumed;
                                        }
                                        Err(_) => {
                                            cur += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // ---- Real Xenos opcodes (values from xenia xenos.h), emitted by the dash''s Direct3D driver ----
                    0x58 => {
                        // EVENT_WRITE_SHD: [initiator, address, value]. Low 2 address bits = endianness of the stored word.
                        if data.len() >= 3 {
                            let initiator = data[0];
                            let address = data[1];
                            let value = if (initiator >> 31) & 1 != 0 { self.gpu_counter } else { data[2] };
                            // The low 2 address bits select the Xenos endian-swap mode used when the GPU writes into a
                            // little-endian host. Guest memory in this emulator is already big-endian (what the PowerPC
                            // lwz sees), so the logical value is stored unchanged. Swapping here wrote 0x03000000 where the
                            // dash polls for 3, which left its fence wait spinning forever.
                            let stored = value;
                            let dst = (address & !3) as u64;
                            let _ = bus.write_u32(dst, stored);
                            // A shader/pixel done event also advances the GPU counter
                            self.gpu_counter = self.gpu_counter.wrapping_add(1);
                        }
                    }
                    0x46 => {
                        // EVENT_WRITE: [initiator] (+ optional address, unused by the dash)
                        if let Some(&init) = data.first() { self.registers.insert(0x21F9, init & 0x3F); }
                    }
                    0x45 => {
                        // COND_WRITE: [wait_info, poll_addr, reference, mask, write_addr, write_data]
                        if data.len() >= 6 {
                            let wait_info = data[0];
                            let is_mem = (wait_info & 0x10) != 0;
                            let cur = if is_mem { bus.read_u32((data[1] & !3) as u64).unwrap_or(0) } else { *self.registers.get(&data[1]).unwrap_or(&0) };
                            let masked = cur & data[3];
                            let ok = match wait_info & 7 {
                                0 => false, 1 => masked < data[2], 2 => masked <= data[2], 3 => masked == data[2],
                                4 => masked != data[2], 5 => masked >= data[2], 6 => masked > data[2], _ => true,
                            };
                            if ok {
                                if (wait_info & 0x100) != 0 { let _ = bus.write_u32((data[4] & !3) as u64, data[5]); }
                                else { self.write_register(data[4], data[5], bus); }
                            }
                        }
                    }
                    0x54 => {
                        // INTERRUPT: raise the graphics interrupt; the vblank callback is driven by the frame loop already
                        self.pending_interrupts += 1;
                    }
                    0x36 | 0x35 => {
                        // DRAW_INDX_2 / DRAW_INDX_2_BIN: [VGT_DRAW_INITIATOR, (indices...)]. Counted; Xenos microcode shaders are not rasterised yet.
                        self.real_draws += 1;
                    }
                    0x22 | 0x34 => {
                        // DRAW_INDX / DRAW_INDX_BIN: [viz_query, VGT_DRAW_INITIATOR, ...]
                        self.real_draws += 1;
                    }
                    0x2B | 0x27 | 0x3B | 0x4A | 0x4B | 0x50 | 0x51 | 0x60 | 0x61 | 0x62 | 0x63 | 0x5E | 0x5D | 0x5C | 0x2E | 0x2C | 0x25 | 0x26 | 0x23 | 0x59 | 0x5A | 0x5B => {
                        // IM_LOAD_IMMEDIATE / IM_LOAD / INVALIDATE_STATE / SET_SHADER_BASES / SET_BIN_* / CONTEXT_UPDATE / WAIT_* /
                        // LOAD_CONSTANT_CONTEXT / IM_STORE / SET_STATE / WAIT_FOR_IDLE / VIZ_QUERY / EVENT_WRITE_CFL|EXT|ZPD:
                        // state-only or synchronisation packets with no observable effect for this software GPU.
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


/// XENON_PM4_LOG=1 prints every PM4 packet the Command Processor executes (cached after first call).
fn pm4_log_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("XENON_PM4_LOG").is_ok())
}
