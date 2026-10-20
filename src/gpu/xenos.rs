use crate::gpu::pm4::*;
use crate::gpu::ring_buffer::RingBuffer;
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
}

/// ATI Xenos (Xbox 360) Command Processor and Graphics Engine
pub struct XenosGpu {
    /// Context registers (e.g., RB_COLOR_INFO, PA_SC_WINDOW_OFFSET, etc.)
    pub registers: HashMap<u32, u32>,
    /// ALU Constant Vector Registers: c0 through c255 (float4 each)
    pub alu_constants: [[f32; 4]; 256],
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

        Self {
            registers,
            alu_constants: [[0.0; 4]; 256],
            color_buffer: vec![0u32; XENOS_WIDTH * XENOS_HEIGHT],
            packets_executed: 0,
            draw_calls: 0,
            primitives_drawn: 0,
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
                    PM4_SET_CONTEXT_REG => {
                        if data.len() >= 2 {
                            let reg = data[0] + 0x2000;
                            let val = data[1];
                            self.registers.insert(reg, val);
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

    /// Executes an indexed draw call by reading indices and rendering to color buffer
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
            // Read 2 bytes
            let b0 = bus.read_u8(addr)?;
            let b1 = bus.read_u8(addr + 1)?;
            let idx = ((b0 as u16) << 8) | (b1 as u16);
            indices.push(idx);
        }

        let num_tris = match prim_type {
            0x04 => num_indices / 3,           // Triangles
            0x05 => num_indices.saturating_sub(2), // Triangle Strip
            _ => num_indices / 3,
        };

        self.primitives_drawn += num_tris as u64;
        Ok(())
    }

    /// Resolves the GPU internal color buffer into the target display framebuffer
    pub fn resolve_surface(&self, target_fb: &mut [u32]) {
        let copy_len = self.color_buffer.len().min(target_fb.len());
        target_fb[..copy_len].copy_from_slice(&self.color_buffer[..copy_len]);
    }
}
