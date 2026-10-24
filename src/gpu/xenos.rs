use crate::gpu::pm4::*;
use crate::gpu::rasterizer::*;
use crate::gpu::ring_buffer::RingBuffer;
use crate::gpu::shader::{
    create_default_xui_pixel_shader, create_default_xui_vertex_shader, PixelShader,
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
    /// GPU counter (Xenia `counter_`): written by EVENT_WRITE_SHD when bit 31 of the initiator is set;
    /// incremented once per vblank (`mark_vblank`) and per XE_SWAP.
    pub gpu_counter: u32,
    /// Draws submitted with the real DRAW_INDX_2 / DRAW_INDX opcodes (not rasterised yet: the dash shaders are Xenos microcode)
    pub real_draws: u64,
    /// CPU numbers of CP interrupts raised by PM4 INTERRUPT packets, not yet delivered to the guest's graphics
    /// interrupt callback (delivered by the frame loop with source = 1).
    pub pending_cp_interrupts: Vec<u32>,
    /// XE_SWAP packets executed (written by VdSwap into the guest command buffer).
    pub swaps: u64,
    /// Last XE_SWAP: (frontbuffer physical address, width, height), as passed to Xenia's IssueSwap.
    pub last_swap: Option<(u32, u32, u32)>,
    pub draw_calls: u64,
    pub primitives_drawn: u64,
    /// Bin mask/select for predicated Type-3 packets (Xenia CommandProcessor bin_mask_/bin_select_, both start at
    /// 0xFFFFFFFF).
    pub bin_mask: u64,
    pub bin_select: u64,
}

impl XenosGpu {
    pub fn new() -> Self {
        let mut alu_constants = [[0.0f32; 4]; 256];
        // Identity MVP matrix in c0..c3
        alu_constants[0] = [1.0, 0.0, 0.0, 0.0];
        alu_constants[1] = [0.0, 1.0, 0.0, 0.0];
        alu_constants[2] = [0.0, 0.0, 1.0, 0.0];
        alu_constants[3] = [0.0, 0.0, 0.0, 1.0];
        // Full opacity diffuse tint in c4
        alu_constants[4] = [1.0, 1.0, 1.0, 1.0];

        Self {
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
            pending_cp_interrupts: Vec::new(),
            swaps: 0,
            last_swap: None,
            draw_calls: 0,
            primitives_drawn: 0,
            bin_mask: 0xFFFF_FFFF,
            bin_select: 0xFFFF_FFFF,
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
            let packet_dwords = Pm4Packet::size_dwords(header);

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

    /// Dispatches a register write (into the shared register file, bus.gpu_regs), updating state and decoding
    /// texture/sampler descriptors
    pub fn write_register(&mut self, reg: u32, val: u32, bus: &mut MemoryBus) {
        bus.gpu_regs.set(reg, val);

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

    /// One vertical blank (Xenia GraphicsSystem::MarkVblank): advances the GPU counter. The frame loop then calls
    /// the guest's interrupt callback with source 0.
    pub fn mark_vblank(&mut self) {
        self.gpu_counter = self.gpu_counter.wrapping_add(1);
    }

    /// Executes one decoded PM4 packet (Xenia ExecutePacketType0..3; opcodes from xenos.h).
    pub fn execute_packet(&mut self, bus: &mut MemoryBus, packet: &Pm4Packet) -> Result<(), String> {
        if pm4_log_enabled() {
            match packet {
                Pm4Packet::Type0 { base_reg, one_reg, values } => println!("  [PM4] T0 base=0x{:04X}{} n={} v0=0x{:08X}", base_reg, if *one_reg { " one-reg" } else { "" }, values.len(), values.first().copied().unwrap_or(0)),
                Pm4Packet::Type1 { reg1, val1, reg2, val2 } => println!("  [PM4] T1 0x{:04X}=0x{:08X} 0x{:04X}=0x{:08X}", reg1, val1, reg2, val2),
                Pm4Packet::Type2Nop => {}
                Pm4Packet::Type3 { opcode, predicated, data } => {
                    let shown: Vec<String> = data.iter().take(8).map(|d| format!("{:08X}", d)).collect();
                    println!("  [PM4] T3 op=0x{:02X} ({}){} n={} data=[{}]", opcode, Pm4Packet::opcode_name(*opcode), if *predicated { " pred" } else { "" }, data.len(), shown.join(" "));
                }
            }
        }
        match packet {
            Pm4Packet::Type0 { base_reg, one_reg, values } => {
                for (i, &val) in values.iter().enumerate() {
                    let reg = if *one_reg { *base_reg } else { base_reg + i as u32 };
                    self.write_register(reg, val, bus);
                }
            }
            Pm4Packet::Type1 { reg1, val1, reg2, val2 } => {
                self.write_register(*reg1, *val1, bus);
                self.write_register(*reg2, *val2, bus);
            }
            Pm4Packet::Type2Nop => {}
            Pm4Packet::Type3 { opcode, predicated, data } => {
                *self.real_opcode_counts.entry(*opcode).or_insert(0) += 1;
                // Predicated packets run only if a selected bin is enabled; predicated swaps never run.
                if *predicated && ((self.bin_select & self.bin_mask) == 0 || *opcode == PM4_XE_SWAP) {
                    return Ok(());
                }
                let word = |i: usize| data.get(i).copied().unwrap_or(0);
                match *opcode {
                    PM4_ME_INIT | PM4_NOP | PM4_WAIT_FOR_IDLE | PM4_WAIT_IB_PFD_COMPLETE | PM4_CONTEXT_UPDATE => {}
                    PM4_INDIRECT_BUFFER | PM4_INDIRECT_BUFFER_PFD => {
                        // [list_ptr, list_length]; length in dwords, 20 bits
                        let ib_addr = word(0) as u64;
                        let ib_count = (word(1) & 0xF_FFFF) as usize;
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
                                    Err(_) => cur += 1,
                                }
                            }
                        }
                    }
                    PM4_REG_RMW => {
                        // [rmw_info, and_mask, or_mask]: reg = rmw_info & 0x1FFF; bit 31 = AND with a register
                        // (and_mask & 0x1FFF) instead of the immediate, bit 30 = same for OR
                        let (info, and_mask, or_mask) = (word(0), word(1), word(2));
                        let reg = info & 0x1FFF;
                        let and_value = if (info >> 31) & 1 != 0 { bus.gpu_regs.get(and_mask & 0x1FFF) } else { and_mask };
                        let or_value = if (info >> 30) & 1 != 0 { bus.gpu_regs.get(or_mask & 0x1FFF) } else { or_mask };
                        let value = (bus.gpu_regs.get(reg) & and_value) | or_value;
                        self.write_register(reg, value, bus);
                    }
                    PM4_REG_TO_MEM => {
                        // [register, address | endian]
                        let value = bus.gpu_regs.get(word(0));
                        gpu_store(bus, word(1), value);
                    }
                    PM4_WAIT_REG_MEM => {
                        // [wait_info, poll_reg_or_addr, reference, mask, wait_interval]
                        if data.len() >= 4 {
                            let wait_info = data[0];
                            let poll_addr = data[1];
                            let reference = data[2];
                            let mask = data[3];

                            let is_mem = (wait_info & 0x10) != 0;
                            let function = wait_info & 0x07;
                            let current_val = if is_mem {
                                gpu_load(bus, poll_addr)
                            } else {
                                bus.gpu_regs.get(poll_addr)
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
                                    gpu_store(bus, poll_addr, forced);
                                } else {
                                    bus.gpu_regs.set(poll_addr, forced);
                                }
                            }
                        }
                    }
                    PM4_MEM_WRITE => {
                        // [address | endian, data...]: count - 1 consecutive dwords
                        let address = word(0);
                        for (i, &v) in data.iter().skip(1).enumerate() {
                            gpu_store(bus, address.wrapping_add(4 * i as u32), v);
                        }
                    }
                    PM4_EVENT_WRITE_SHD => {
                        // EVENT_WRITE_SHD: [initiator, address, value]. Low 2 address bits = endianness of the stored word.
                        let (initiator, address) = (word(0), word(1));
                        bus.gpu_regs.set(VGT_EVENT_INITIATOR, initiator & 0x3F);
                        let value = if (initiator >> 31) & 1 != 0 { self.gpu_counter } else { word(2) };
                        // Addresses above 0x1FFFFFFF inside the writeback window (WRITEBACK_START/SIZE, once the
                        // guest set them) land at virtual 0x7F000000 + offset.
                        let writeback_base = bus.gpu_regs.get(WRITEBACK_START);
                        let offset = (address & !3).wrapping_sub(writeback_base);
                        if address & !3 > 0x1FFF_FFFF && writeback_base != 0 && offset < bus.gpu_regs.get(WRITEBACK_SIZE) {
                            let _ = bus.write_u32(0x7F00_0000 + offset as u64, guest_endian_swap(value, address));
                        } else {
                            gpu_store(bus, address, value);
                        }
                    }
                    PM4_EVENT_WRITE => {
                        // EVENT_WRITE: [initiator] (+ optional address, unused by the dash)
                        bus.gpu_regs.set(VGT_EVENT_INITIATOR, word(0) & 0x3F);
                    }
                    PM4_COND_WRITE => {
                        // COND_WRITE: [wait_info, poll_addr, reference, mask, write_addr, write_data]
                        if data.len() >= 6 {
                            let wait_info = data[0];
                            let is_mem = (wait_info & 0x10) != 0;
                            let cur = if is_mem { gpu_load(bus, data[1]) } else { bus.gpu_regs.get(data[1]) };
                            let masked = cur & data[3];
                            let ok = match wait_info & 7 {
                                0 => false, 1 => masked < data[2], 2 => masked <= data[2], 3 => masked == data[2],
                                4 => masked != data[2], 5 => masked >= data[2], 6 => masked > data[2], _ => true,
                            };
                            if ok {
                                if (wait_info & 0x100) != 0 { gpu_store(bus, data[4], data[5]); }
                                else { self.write_register(data[4], data[5], bus); }
                            }
                        }
                    }
                    PM4_INTERRUPT => {
                        // INTERRUPT: [cpu_mask]; one CP interrupt (callback source 1) per set bit 0..5
                        // (Xenia ExecutePacketType3_INTERRUPT -> DispatchInterruptCallback(1, n)).
                        let cpu_mask = word(0);
                        for n in 0..6 {
                            if cpu_mask & (1 << n) != 0 {
                                self.pending_cp_interrupts.push(n);
                            }
                        }
                    }
                    PM4_XE_SWAP => {
                        // XE_SWAP (Xenia ExecutePacketType3_XE_SWAP): [signature 'SWAP', frontbuffer physical, width,
                        // height, ...]; issues the swap and advances the GPU counter.
                        if data.first() == Some(&crate::kernel::video::SWAP_SIGNATURE) && data.len() >= 4 {
                            self.last_swap = Some((data[1], data[2], data[3]));
                            self.swaps += 1;
                        }
                        self.gpu_counter = self.gpu_counter.wrapping_add(1);
                    }
                    PM4_DRAW_INDX | PM4_DRAW_INDX_2 | PM4_DRAW_INDX_BIN | PM4_DRAW_INDX_2_BIN => {
                        // Counted; Xenos microcode shaders are not rasterised yet (Phase 4).
                        self.real_draws += 1;
                    }
                    PM4_SET_CONSTANT => {
                        // [offset_type, values...]: index = bits 0..10, type = bits 16..23
                        let offset_type = word(0);
                        if let Some(base) = constant_register_base((offset_type >> 16) & 0xFF) {
                            let first = base + (offset_type & 0x7FF);
                            for (i, &v) in data.iter().skip(1).enumerate() {
                                self.write_register(first + i as u32, v, bus);
                            }
                        }
                    }
                    PM4_SET_CONSTANT2 | PM4_SET_SHADER_CONSTANTS => {
                        // [offset_type, values...]: register index = low 16 bits
                        let first = word(0) & 0xFFFF;
                        for (i, &v) in data.iter().skip(1).enumerate() {
                            self.write_register(first + i as u32, v, bus);
                        }
                    }
                    PM4_LOAD_ALU_CONSTANT => {
                        // [address (physical), offset_type, size_dwords]: registers loaded from memory
                        let address = (word(0) & 0x3FFF_FFFF) as u64;
                        let offset_type = word(1);
                        let size = word(2) & 0xFFF;
                        if let Some(base) = constant_register_base((offset_type >> 16) & 0xFF) {
                            let first = base + (offset_type & 0x7FF);
                            for i in 0..size {
                                let v = bus.read_u32(address + 4 * i as u64).unwrap_or(0);
                                self.write_register(first + i, v, bus);
                            }
                        }
                    }
                    PM4_SET_BIN_MASK_LO => self.bin_mask = (self.bin_mask & !0xFFFF_FFFF) | word(0) as u64,
                    PM4_SET_BIN_MASK_HI => self.bin_mask = (self.bin_mask & 0xFFFF_FFFF) | (word(0) as u64) << 32,
                    PM4_SET_BIN_SELECT_LO => self.bin_select = (self.bin_select & !0xFFFF_FFFF) | word(0) as u64,
                    PM4_SET_BIN_SELECT_HI => self.bin_select = (self.bin_select & 0xFFFF_FFFF) | (word(0) as u64) << 32,
                    PM4_SET_BIN_MASK => self.bin_mask = (word(0) as u64) << 32 | word(1) as u64,
                    PM4_SET_BIN_SELECT => self.bin_select = (word(0) as u64) << 32 | word(1) as u64,
                    PM4_IM_LOAD | PM4_IM_LOAD_IMMEDIATE | PM4_INVALIDATE_STATE | PM4_VIZ_QUERY | PM4_EVENT_WRITE_EXT
                    | PM4_EVENT_WRITE_ZPD => {
                        // Implemented by Xenia but with no effect yet on this software GPU (shader upload, viz queries,
                        // extents, z-pass counts: Phase 4).
                    }
                    _ => {
                        // Unimplemented in Xenia too (HitUnimplementedOpcode); counted in real_opcode_counts.
                    }
                }
            }
        }
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
const VGT_EVENT_INITIATOR: u32 = 0x21F9;
const WRITEBACK_START: u32 = 0x0A04;
const WRITEBACK_SIZE: u32 = 0x0A05;

/// A 32-bit value the GPU moves between memory and itself with endian mode `endian & 3` (xenos::Endian, the low
/// address bits), as the big-endian guest sees it. Xenia stores GpuSwap(v, endian) with a little-endian host store
/// (and loads the same way), so through lwz/stw: kNone = byte swap, k8in16 = half-words swapped, k8in32 = unchanged,
/// k16in32 = bytes swapped inside each half-word. Each mode is its own inverse.
fn guest_endian_swap(value: u32, endian: u32) -> u32 {
    match endian & 3 {
        0 => value.swap_bytes(),
        1 => value.rotate_left(16),
        2 => value,
        _ => ((value << 8) & 0xFF00_FF00) | ((value >> 8) & 0x00FF_00FF),
    }
}

/// GPU store to `address` (physical; bits 0..1 = endian mode), as Xenia's TranslatePhysical + GpuSwap.
fn gpu_store(bus: &mut MemoryBus, address: u32, value: u32) {
    let _ = bus.write_u32((address & 0x1FFF_FFFC) as u64, guest_endian_swap(value, address));
}

/// GPU load from `address` (physical; bits 0..1 = endian mode).
fn gpu_load(bus: &MemoryBus, address: u32) -> u32 {
    guest_endian_swap(bus.read_u32((address & 0x1FFF_FFFC) as u64).unwrap_or(0), address)
}

/// First register of a SET_CONSTANT / LOAD_ALU_CONSTANT type (Xenia Write{ALU,Fetch,Bool,Loop,REGISTERS}Range*).
fn constant_register_base(kind: u32) -> Option<u32> {
    match kind {
        0 => Some(0x4000), // ALU
        1 => Some(0x4800), // FETCH
        2 => Some(0x4900), // BOOL
        3 => Some(0x4908), // LOOP
        4 => Some(0x2000), // REGISTERS
        _ => None,
    }
}

fn pm4_log_enabled() -> bool {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::diag::config().pm4_log)
}

#[cfg(test)]
mod cp_tests {
    use super::*;

    fn t3(opcode: u8, data: &[u32]) -> Pm4Packet {
        Pm4Packet::Type3 { opcode, predicated: false, data: data.to_vec() }
    }

    #[test]
    fn interrupt_queues_one_cp_interrupt_per_cpu_bit() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        gpu.execute_packet(&mut bus, &t3(0x54, &[0b10_0101])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts, vec![0, 2, 5]);
        // bits above 5 are ignored (six hardware threads)
        gpu.execute_packet(&mut bus, &t3(0x54, &[0xC0])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts, vec![0, 2, 5]);
    }

    #[test]
    fn cp_and_mmio_share_one_register_file() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // Type-0: two registers from 0x2000 (a register outside the fixed MMIO ones)
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x2000, one_reg: false, values: vec![0x1234, 0x5678] }).unwrap();
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x2000 * 4).unwrap(), 0x1234);
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x2001 * 4).unwrap(), 0x5678);
        // a CPU MMIO store is what REG_RMW (0x21) reads back: (old & and) | or
        bus.write_u32(0x7FC8_0000 + 0x0200 * 4, 0xFF00).unwrap();
        gpu.execute_packet(&mut bus, &t3(0x21, &[0x0200, 0x0F00, 0x0003])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x0200), 0x0F03);
        assert_eq!(bus.read_u32(0x7FC8_0800).unwrap(), 0x0F03);
    }

    #[test]
    fn type0_one_reg_writes_every_value_to_the_base_register() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        gpu.execute_packet(&mut bus, &Pm4Packet::Type0 { base_reg: 0x2100, one_reg: true, values: vec![1, 2, 3] }).unwrap();
        assert_eq!((bus.gpu_regs.get(0x2100), bus.gpu_regs.get(0x2101)), (3, 0));
    }

    #[test]
    fn predicated_packets_follow_bin_select_and_mask() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        let pred = |opcode: u8, data: &[u32]| Pm4Packet::Type3 { opcode, predicated: true, data: data.to_vec() };
        // default mask/select 0xFFFFFFFF: predicated INTERRUPT runs, predicated XE_SWAP never does
        gpu.execute_packet(&mut bus, &pred(PM4_INTERRUPT, &[1])).unwrap();
        gpu.execute_packet(&mut bus, &pred(PM4_XE_SWAP, &[0x5357_4150, 0, 1, 1])).unwrap();
        assert_eq!((gpu.pending_cp_interrupts.len(), gpu.swaps, gpu.gpu_counter), (1, 0, 0));
        // SET_BIN_SELECT (hi, lo) disjoint from the mask: skipped
        gpu.execute_packet(&mut bus, &t3(PM4_SET_BIN_SELECT, &[1, 0])).unwrap();
        assert_eq!(gpu.bin_select, 1 << 32);
        gpu.execute_packet(&mut bus, &pred(PM4_INTERRUPT, &[1])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts.len(), 1);
        gpu.execute_packet(&mut bus, &t3(PM4_SET_BIN_MASK_HI, &[1])).unwrap();
        gpu.execute_packet(&mut bus, &pred(PM4_INTERRUPT, &[1])).unwrap();
        assert_eq!(gpu.pending_cp_interrupts.len(), 2);
    }

    #[test]
    fn constants_land_in_the_register_file() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // SET_CONSTANT type 0 (ALU) index 8 -> 0x4008; type 1 (FETCH) index 2 -> 0x4802; type 4 -> 0x2000 + index
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT, &[0x0000_0008, 0xA, 0xB])).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT, &[0x0001_0002, 0xC])).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT, &[0x0004_0081, 0xD])).unwrap();
        assert_eq!((bus.gpu_regs.get(0x4008), bus.gpu_regs.get(0x4009), bus.gpu_regs.get(0x4802), bus.gpu_regs.get(0x2081)), (0xA, 0xB, 0xC, 0xD));
        // LOAD_ALU_CONSTANT: [physical address, offset_type, size] -> registers from memory
        bus.write_u32(0x0100_0000, 0x11).unwrap();
        bus.write_u32(0x0100_0004, 0x22).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_LOAD_ALU_CONSTANT, &[0x0100_0000, 0x0000_0010, 2])).unwrap();
        assert_eq!((bus.gpu_regs.get(0x4010), bus.gpu_regs.get(0x4011)), (0x11, 0x22));
        // SET_CONSTANT2: register index in the low 16 bits
        gpu.execute_packet(&mut bus, &t3(PM4_SET_CONSTANT2, &[0x0000_2300, 0xE])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x2300), 0xE);
        // the old invented opcode 0x22 is DRAW_INDX: counted as a draw, writes no register
        let before = bus.gpu_regs.non_zero().count();
        gpu.execute_packet(&mut bus, &t3(PM4_DRAW_INDX, &[0x84, 0x1234])).unwrap();
        assert_eq!((gpu.real_draws, bus.gpu_regs.non_zero().count()), (1, before));
    }

    #[test]
    fn gpu_memory_writes_honor_the_endian_mode() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        const A: u32 = 0x0100_0000;
        // MEM_WRITE: N dwords from the address; endian 2 (k8in32) keeps the value as lwz sees it
        gpu.execute_packet(&mut bus, &t3(PM4_MEM_WRITE, &[A | 2, 0xAABB_CCDD, 0x1122_3344, 0x5566_7788])).unwrap();
        assert_eq!((bus.read_u32(A as u64).unwrap(), bus.read_u32(A as u64 + 8).unwrap()), (0xAABB_CCDD, 0x5566_7788));
        // other modes: none = byte swap, k8in16 = halves swapped, k16in32 = bytes swapped inside halves
        for (endian, seen) in [(0, 0xDDCC_BBAA), (1, 0xCCDD_AABB), (3, 0xBBAA_DDCC)] {
            gpu.execute_packet(&mut bus, &t3(PM4_MEM_WRITE, &[A | endian, 0xAABB_CCDD])).unwrap();
            assert_eq!(bus.read_u32(A as u64).unwrap(), seen, "endian {}", endian);
            assert_eq!(gpu_load(&bus, A | endian), 0xAABB_CCDD, "loads undo the same mode");
        }
        // COND_WRITE polls memory with endian 0 and writes memory with endian 0
        bus.write_u32(A as u64, 0x0300_0000).unwrap();
        gpu.execute_packet(&mut bus, &t3(PM4_COND_WRITE, &[0x113, A, 3, 0xFF, (A + 4), 7])).unwrap();
        assert_eq!(bus.read_u32(A as u64 + 4).unwrap(), 0x0700_0000);
        // EVENT_WRITE_SHD with endian 0, and the initiator lands in VGT_EVENT_INITIATOR
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0x14, A + 8, 9])).unwrap();
        assert_eq!((bus.read_u32(A as u64 + 8).unwrap(), bus.gpu_regs.get(0x21F9)), (0x0900_0000, 0x14));
        // REG_TO_MEM
        bus.gpu_regs.set(0x2000, 0x1234_5678);
        gpu.execute_packet(&mut bus, &t3(PM4_REG_TO_MEM, &[0x2000, (A + 12) | 2])).unwrap();
        assert_eq!(bus.read_u32(A as u64 + 12).unwrap(), 0x1234_5678);
    }

    #[test]
    fn event_write_shd_uses_the_writeback_window() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        // no window set: an address above 0x1FFFFFFF goes to its physical alias
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0, 0xC000_0102, 5])).unwrap();
        assert_eq!(bus.read_u32(0x0000_0100).unwrap(), 5);
        bus.gpu_regs.set(0x0A04, 0xC000_0000);
        bus.gpu_regs.set(0x0A05, 0x1000);
        gpu.execute_packet(&mut bus, &t3(PM4_EVENT_WRITE_SHD, &[0, 0xC000_0042, 6])).unwrap();
        assert_eq!(bus.read_u32(0x7F00_0040).unwrap(), 6);
    }

    #[test]
    fn reg_rmw_register_operands() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(0x100, 0xFFFF);
        bus.gpu_regs.set(0x101, 0x0F0F);
        bus.gpu_regs.set(0x102, 0x3000_0000);
        // AND with register 0x101, OR with register 0x102
        gpu.execute_packet(&mut bus, &t3(PM4_REG_RMW, &[0xC000_0100, 0x101, 0x102])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x100), 0x3000_0F0F);
        // immediates
        gpu.execute_packet(&mut bus, &t3(PM4_REG_RMW, &[0x100, 0xFF, 0x100])).unwrap();
        assert_eq!(bus.gpu_regs.get(0x100), 0x10F);
    }

    #[test]
    fn counter_advances_on_vblank_and_swap_not_on_event_write_shd() {
        let mut gpu = XenosGpu::new();
        let mut bus = MemoryBus::new();
        const FENCE: u64 = 0x1010_2000;
        // the dash's fence write: [initiator 3, address | endian 2, value]
        gpu.execute_packet(&mut bus, &t3(0x58, &[3, 0x1010_2002, 0xB])).unwrap();
        assert_eq!((bus.read_u32(FENCE).unwrap(), gpu.gpu_counter), (0xB, 0));
        gpu.mark_vblank();
        gpu.mark_vblank();
        gpu.execute_packet(&mut bus, &t3(0x64, &[0x5357_4150, 0x1031_4000, 1280, 720])).unwrap();
        assert_eq!(gpu.gpu_counter, 3);
        assert_eq!((gpu.swaps, gpu.last_swap), (1, Some((0x1031_4000, 1280, 720))));
        // initiator bit 31 writes the counter instead of the value
        gpu.execute_packet(&mut bus, &t3(0x58, &[0x8000_0003, 0x1010_2002, 0xDEAD])).unwrap();
        assert_eq!((bus.read_u32(FENCE).unwrap(), gpu.gpu_counter), (3, 3));
    }
}
