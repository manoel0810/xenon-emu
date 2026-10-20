// =========================================================================
// ATI Xenos (Xbox 360) PM4 Packet Types and Opcodes
// =========================================================================

pub const PM4_TYPE_0: u32 = 0x00; // Register Write
pub const PM4_TYPE_1: u32 = 0x01; // Register Pair Write
pub const PM4_TYPE_2: u32 = 0x02; // No-op / Padding
pub const PM4_TYPE_3: u32 = 0x03; // Command Processor Opcode

// PM4 Type 3 Opcodes used by Xbox 360 DirectX 9 / XUI Subsystem
pub const PM4_NOP: u8 = 0x10;
pub const PM4_SET_CONSTANT: u8 = 0x21;
pub const PM4_SET_CONTEXT_REG: u8 = 0x22;
pub const PM4_DRAW_INDX: u8 = 0x23;
pub const PM4_DRAW_INDX_2: u8 = 0x24;
pub const PM4_INDEX_TYPE: u8 = 0x2A;
pub const PM4_LOAD_ALU_CONSTANT: u8 = 0x2F;
pub const PM4_WAIT_FOR_IDLE: u8 = 0x34;
pub const PM4_MEM_WRITE: u8 = 0x38;
pub const PM4_EXECUTE_INDIRECT: u8 = 0x3F;
pub const PM4_ME_INIT: u8 = 0x48;

// Key Xenos Registers
pub const RB_COLOR_INFO: u32 = 0x2004;
pub const RB_DEPTH_INFO: u32 = 0x2008;
pub const PA_SC_WINDOW_OFFSET: u32 = 0x2080;
pub const PA_CL_VPORT_XSCALE: u32 = 0x2084;
pub const PA_CL_VPORT_YSCALE: u32 = 0x2088;
pub const PA_CL_VPORT_XOFFSET: u32 = 0x208C;
pub const PA_CL_VPORT_YOFFSET: u32 = 0x2090;
pub const SQ_PROGRAM_CNTL: u32 = 0x2180;
pub const SQ_CONTEXT_MISC: u32 = 0x2181;
pub const SQ_INTERPOLATOR_CNTL: u32 = 0x2182;
pub const SQ_VS_PROGRAM_ADDR: u32 = 0x2183;
pub const SQ_PS_PROGRAM_ADDR: u32 = 0x2184;
pub const SQ_VS_CONST_BASE: u32 = 0x2185;
pub const SQ_PS_CONST_BASE: u32 = 0x2186;
pub const VGT_DRAW_INITIATOR: u32 = 0x2191;
pub const VGT_DMA_BASE: u32 = 0x2192;
pub const VGT_DMA_SIZE: u32 = 0x2193;
pub const VGT_DMA_STRIDE: u32 = 0x2194;
pub const PA_SU_SC_MODE_CNTL: u32 = 0x2205;
pub const SQ_TEX_RESOURCE_BASE: u32 = 0x0A00;
pub const SQ_TEX_RESOURCE_END: u32 = 0x0A60;
pub const SQ_TEX_SAMPLER_BASE: u32 = 0x0D00;
pub const SQ_TEX_SAMPLER_END: u32 = 0x0D10;

/// High-level representation of a decoded ATI Xenos PM4 Packet
#[derive(Debug, Clone)]
pub enum Pm4Packet {
    Type0 {
        base_reg: u32,
        count: usize,
        values: Vec<u32>,
    },
    Type1 {
        reg1: u32,
        val1: u32,
        reg2: u32,
        val2: u32,
    },
    Type2Nop {
        count: usize,
    },
    Type3 {
        opcode: u8,
        count: usize,
        data: Vec<u32>,
    },
}

impl Pm4Packet {
    /// Decodes a PM4 packet from a 32-bit slice. Returns (Packet, total_dwords_consumed)
    pub fn decode(words: &[u32]) -> Result<(Self, usize), String> {
        if words.is_empty() {
            return Err("Empty buffer for PM4 packet".into());
        }

        let header = words[0];
        let ptype = (header >> 30) & 0x03;

        match ptype {
            PM4_TYPE_0 => {
                let base_reg = header & 0x7FFF;
                let count = (((header >> 16) & 0x3FFF) + 1) as usize;
                if words.len() < 1 + count {
                    return Err(format!("Type 0 packet incomplete: need {} words, got {}", count + 1, words.len()));
                }
                let values = words[1..=count].to_vec();
                Ok((Pm4Packet::Type0 { base_reg, count, values }, count + 1))
            }
            PM4_TYPE_1 => {
                if words.len() < 3 {
                    return Err("Type 1 packet incomplete".into());
                }
                let reg1 = header & 0x7FF;
                let reg2 = (header >> 11) & 0x7FF;
                let val1 = words[1];
                let val2 = words[2];
                Ok((Pm4Packet::Type1 { reg1, val1, reg2, val2 }, 3))
            }
            PM4_TYPE_2 => {
                // NOP packet
                Ok((Pm4Packet::Type2Nop { count: 1 }, 1))
            }
            PM4_TYPE_3 => {
                let opcode = ((header >> 8) & 0xFF) as u8;
                let count = (((header >> 16) & 0x3FFF) + 1) as usize;
                if words.len() < 1 + count {
                    return Err(format!("Type 3 packet (op 0x{:02X}) incomplete: need {} words, got {}", opcode, count + 1, words.len()));
                }
                let data = words[1..=count].to_vec();
                Ok((Pm4Packet::Type3 { opcode, count, data }, count + 1))
            }
            _ => Err(format!("Unknown PM4 packet type: {}", ptype)),
        }
    }

    /// Returns a human-readable opcode name for Type 3 packets
    pub fn opcode_name(opcode: u8) -> &'static str {
        match opcode {
            PM4_NOP => "PM4_NOP",
            PM4_SET_CONSTANT => "PM4_SET_CONSTANT",
            PM4_SET_CONTEXT_REG => "PM4_SET_CONTEXT_REG",
            PM4_DRAW_INDX => "PM4_DRAW_INDX",
            PM4_DRAW_INDX_2 => "PM4_DRAW_INDX_2",
            PM4_INDEX_TYPE => "PM4_INDEX_TYPE",
            PM4_LOAD_ALU_CONSTANT => "PM4_LOAD_ALU_CONSTANT",
            PM4_WAIT_FOR_IDLE => "PM4_WAIT_FOR_IDLE",
            PM4_MEM_WRITE => "PM4_MEM_WRITE",
            PM4_EXECUTE_INDIRECT => "PM4_EXECUTE_INDIRECT",
            PM4_ME_INIT => "PM4_ME_INIT",
            _ => "PM4_UNKNOWN_OPCODE",
        }
    }
}

// =========================================================================
// Packet Builders (Used by CPU D3D / XUI to emit commands to the GPU)
// =========================================================================

/// Builds a PM4 Type 0 Packet (Writes consecutive registers)
pub fn build_type0(base_reg: u32, values: &[u32]) -> Vec<u32> {
    assert!(!values.is_empty(), "Values must not be empty");
    let count = (values.len() - 1) as u32;
    let header = (PM4_TYPE_0 << 30) | (count << 16) | (base_reg & 0x7FFF);
    let mut pkt = Vec::with_capacity(values.len() + 1);
    pkt.push(header);
    pkt.extend_from_slice(values);
    pkt
}

/// Builds a PM4 Type 3 Packet (Opcode + payload)
pub fn build_type3(opcode: u8, data: &[u32]) -> Vec<u32> {
    assert!(!data.is_empty(), "Data must not be empty");
    let count = (data.len() - 1) as u32;
    let header = (PM4_TYPE_3 << 30) | (count << 16) | ((opcode as u32) << 8);
    let mut pkt = Vec::with_capacity(data.len() + 1);
    pkt.push(header);
    pkt.extend_from_slice(data);
    pkt
}

/// Helper to build PM4_SET_CONTEXT_REG (Opcode 0x22)
pub fn build_set_context_reg(reg: u32, value: u32) -> Vec<u32> {
    // Word 0 in payload is (reg_offset - 0x2000)
    let reg_offset = reg.saturating_sub(0x2000);
    build_type3(PM4_SET_CONTEXT_REG, &[reg_offset, value])
}

/// Helper to build PM4_LOAD_ALU_CONSTANT (Opcode 0x2F)
pub fn build_load_alu_constant(start_idx: u32, vectors: &[[f32; 4]]) -> Vec<u32> {
    let mut payload = Vec::with_capacity(1 + vectors.len() * 4);
    payload.push(start_idx); // Starting register index (0..255)
    for v in vectors {
        payload.push(v[0].to_bits());
        payload.push(v[1].to_bits());
        payload.push(v[2].to_bits());
        payload.push(v[3].to_bits());
    }
    build_type3(PM4_LOAD_ALU_CONSTANT, &payload)
}

/// Helper to build PM4_DRAW_INDX (Opcode 0x23)
pub fn build_draw_indexed(prim_type: u32, num_indices: u32, index_ptr: u32) -> Vec<u32> {
    // Payload: [0: Dword0 (VGT DMA), 1: NumIndices, 2: IndexBasePtr, 3: IndexSize]
    build_type3(PM4_DRAW_INDX, &[prim_type, num_indices, index_ptr, 0x0002])
}

/// Helper to build PM4_SET_CONSTANT (Opcode 0x21)
pub fn build_set_constant(reg_idx: u32, values: &[u32]) -> Vec<u32> {
    let mut payload = Vec::with_capacity(1 + values.len());
    payload.push(reg_idx);
    payload.extend_from_slice(values);
    build_type3(PM4_SET_CONSTANT, &payload)
}

/// Helper to bind a Vertex Shader or Pixel Shader program address
pub fn build_set_shader_program(is_vertex_shader: bool, program_addr: u32) -> Vec<u32> {
    let reg = if is_vertex_shader { SQ_VS_PROGRAM_ADDR } else { SQ_PS_PROGRAM_ADDR };
    build_set_context_reg(reg, program_addr)
}

/// Helper to build PM4_WAIT_FOR_IDLE (Opcode 0x34)
pub fn build_wait_for_idle() -> Vec<u32> {
    build_type3(PM4_WAIT_FOR_IDLE, &[0x00000000])
}

