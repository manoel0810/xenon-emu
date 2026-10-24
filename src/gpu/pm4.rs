// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/xenos.h                          enum Type3Opcode, MakePacketType0..3
//   src/xenia/gpu/pm4_command_processor_implement.h ExecutePacketType0/1/2/3 (header decoding)
//
// =========================================================================
// ATI Xenos (Xbox 360) PM4 Packet Types and Opcodes
// =========================================================================

pub const PM4_TYPE_0: u32 = 0x00; // Register Write
pub const PM4_TYPE_1: u32 = 0x01; // Register Pair Write
pub const PM4_TYPE_2: u32 = 0x02; // No-op / Padding
pub const PM4_TYPE_3: u32 = 0x03; // Command Processor Opcode

// Type-3 opcodes (IT_OPCODE), xenos.h `Type3Opcode`.
pub const PM4_ME_INIT: u8 = 0x48;
pub const PM4_NOP: u8 = 0x10;
pub const PM4_INDIRECT_BUFFER: u8 = 0x3F;
pub const PM4_INDIRECT_BUFFER_PFD: u8 = 0x37;
pub const PM4_WAIT_FOR_IDLE: u8 = 0x26;
pub const PM4_WAIT_REG_MEM: u8 = 0x3C;
pub const PM4_WAIT_REG_EQ: u8 = 0x52;
pub const PM4_WAIT_REG_GTE: u8 = 0x53;
pub const PM4_WAIT_UNTIL_READ: u8 = 0x5C;
pub const PM4_WAIT_IB_PFD_COMPLETE: u8 = 0x5D;
pub const PM4_REG_RMW: u8 = 0x21;
pub const PM4_REG_TO_MEM: u8 = 0x3E;
pub const PM4_MEM_WRITE: u8 = 0x3D;
pub const PM4_MEM_WRITE_CNTR: u8 = 0x4F;
pub const PM4_COND_EXEC: u8 = 0x44;
pub const PM4_COND_WRITE: u8 = 0x45;
pub const PM4_EVENT_WRITE: u8 = 0x46;
pub const PM4_EVENT_WRITE_SHD: u8 = 0x58;
pub const PM4_EVENT_WRITE_CFL: u8 = 0x59;
pub const PM4_EVENT_WRITE_EXT: u8 = 0x5A;
pub const PM4_EVENT_WRITE_ZPD: u8 = 0x5B;
pub const PM4_DRAW_INDX: u8 = 0x22;
pub const PM4_DRAW_INDX_2: u8 = 0x36;
pub const PM4_DRAW_INDX_BIN: u8 = 0x34;
pub const PM4_DRAW_INDX_2_BIN: u8 = 0x35;
pub const PM4_VIZ_QUERY: u8 = 0x23;
pub const PM4_SET_STATE: u8 = 0x25;
pub const PM4_SET_CONSTANT: u8 = 0x2D;
pub const PM4_SET_CONSTANT2: u8 = 0x55;
pub const PM4_SET_SHADER_CONSTANTS: u8 = 0x56;
pub const PM4_LOAD_ALU_CONSTANT: u8 = 0x2F;
pub const PM4_IM_LOAD: u8 = 0x27;
pub const PM4_IM_LOAD_IMMEDIATE: u8 = 0x2B;
pub const PM4_LOAD_CONSTANT_CONTEXT: u8 = 0x2E;
pub const PM4_INVALIDATE_STATE: u8 = 0x3B;
pub const PM4_SET_SHADER_BASES: u8 = 0x4A;
pub const PM4_SET_BIN_BASE_OFFSET: u8 = 0x4B;
pub const PM4_SET_BIN_MASK: u8 = 0x50;
pub const PM4_SET_BIN_SELECT: u8 = 0x51;
pub const PM4_CONTEXT_UPDATE: u8 = 0x5E;
pub const PM4_INTERRUPT: u8 = 0x54;
pub const PM4_XE_SWAP: u8 = 0x64; // Xenia only: written by VdSwap
pub const PM4_IM_STORE: u8 = 0x2C;
pub const PM4_SET_BIN_MASK_LO: u8 = 0x60;
pub const PM4_SET_BIN_MASK_HI: u8 = 0x61;
pub const PM4_SET_BIN_SELECT_LO: u8 = 0x62;
pub const PM4_SET_BIN_SELECT_HI: u8 = 0x63;

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
    /// Writes `values` to consecutive registers from `base_reg`, or all to `base_reg` when `one_reg` (bit 15).
    Type0 {
        base_reg: u32,
        one_reg: bool,
        values: Vec<u32>,
    },
    Type1 {
        reg1: u32,
        val1: u32,
        reg2: u32,
        val2: u32,
    },
    Type2Nop,
    /// `predicated` (header bit 0): executed only if bin_select & bin_mask != 0.
    Type3 {
        opcode: u8,
        predicated: bool,
        data: Vec<u32>,
    },
}

impl Pm4Packet {
    /// Total dwords (header included) of the packet starting with `header`.
    pub fn size_dwords(header: u32) -> usize {
        match (header >> 30) & 3 {
            PM4_TYPE_0 | PM4_TYPE_3 => (((header >> 16) & 0x3FFF) + 2) as usize,
            PM4_TYPE_1 => 3,
            _ => 1,
        }
    }

    /// Decodes a PM4 packet from a 32-bit slice. Returns (Packet, total_dwords_consumed)
    pub fn decode(words: &[u32]) -> Result<(Self, usize), String> {
        let Some(&header) = words.first() else {
            return Err("Empty buffer for PM4 packet".into());
        };
        let size = Self::size_dwords(header);
        if words.len() < size {
            return Err(format!("PM4 packet {:08X} incomplete: need {} words, got {}", header, size, words.len()));
        }
        let body = &words[1..size];
        let packet = match (header >> 30) & 3 {
            PM4_TYPE_0 => Pm4Packet::Type0 { base_reg: header & 0x7FFF, one_reg: (header >> 15) & 1 != 0, values: body.to_vec() },
            PM4_TYPE_1 => Pm4Packet::Type1 { reg1: header & 0x7FF, val1: body[0], reg2: (header >> 11) & 0x7FF, val2: body[1] },
            PM4_TYPE_2 => Pm4Packet::Type2Nop,
            _ => Pm4Packet::Type3 { opcode: ((header >> 8) & 0x7F) as u8, predicated: header & 1 != 0, data: body.to_vec() },
        };
        Ok((packet, size))
    }

    /// Name of a Type-3 opcode (xenos.h), or "UNKNOWN".
    pub fn opcode_name(opcode: u8) -> &'static str {
        match opcode {
            PM4_ME_INIT => "ME_INIT",
            PM4_NOP => "NOP",
            PM4_INDIRECT_BUFFER => "INDIRECT_BUFFER",
            PM4_INDIRECT_BUFFER_PFD => "INDIRECT_BUFFER_PFD",
            PM4_WAIT_FOR_IDLE => "WAIT_FOR_IDLE",
            PM4_WAIT_REG_MEM => "WAIT_REG_MEM",
            PM4_WAIT_REG_EQ => "WAIT_REG_EQ",
            PM4_WAIT_REG_GTE => "WAIT_REG_GTE",
            PM4_WAIT_UNTIL_READ => "WAIT_UNTIL_READ",
            PM4_WAIT_IB_PFD_COMPLETE => "WAIT_IB_PFD_COMPLETE",
            PM4_REG_RMW => "REG_RMW",
            PM4_REG_TO_MEM => "REG_TO_MEM",
            PM4_MEM_WRITE => "MEM_WRITE",
            PM4_MEM_WRITE_CNTR => "MEM_WRITE_CNTR",
            PM4_COND_EXEC => "COND_EXEC",
            PM4_COND_WRITE => "COND_WRITE",
            PM4_EVENT_WRITE => "EVENT_WRITE",
            PM4_EVENT_WRITE_SHD => "EVENT_WRITE_SHD",
            PM4_EVENT_WRITE_CFL => "EVENT_WRITE_CFL",
            PM4_EVENT_WRITE_EXT => "EVENT_WRITE_EXT",
            PM4_EVENT_WRITE_ZPD => "EVENT_WRITE_ZPD",
            PM4_DRAW_INDX => "DRAW_INDX",
            PM4_DRAW_INDX_2 => "DRAW_INDX_2",
            PM4_DRAW_INDX_BIN => "DRAW_INDX_BIN",
            PM4_DRAW_INDX_2_BIN => "DRAW_INDX_2_BIN",
            PM4_VIZ_QUERY => "VIZ_QUERY",
            PM4_SET_STATE => "SET_STATE",
            PM4_SET_CONSTANT => "SET_CONSTANT",
            PM4_SET_CONSTANT2 => "SET_CONSTANT2",
            PM4_SET_SHADER_CONSTANTS => "SET_SHADER_CONSTANTS",
            PM4_LOAD_ALU_CONSTANT => "LOAD_ALU_CONSTANT",
            PM4_IM_LOAD => "IM_LOAD",
            PM4_IM_LOAD_IMMEDIATE => "IM_LOAD_IMMEDIATE",
            PM4_LOAD_CONSTANT_CONTEXT => "LOAD_CONSTANT_CONTEXT",
            PM4_INVALIDATE_STATE => "INVALIDATE_STATE",
            PM4_SET_SHADER_BASES => "SET_SHADER_BASES",
            PM4_SET_BIN_BASE_OFFSET => "SET_BIN_BASE_OFFSET",
            PM4_SET_BIN_MASK => "SET_BIN_MASK",
            PM4_SET_BIN_SELECT => "SET_BIN_SELECT",
            PM4_CONTEXT_UPDATE => "CONTEXT_UPDATE",
            PM4_INTERRUPT => "INTERRUPT",
            PM4_XE_SWAP => "XE_SWAP",
            PM4_IM_STORE => "IM_STORE",
            PM4_SET_BIN_MASK_LO => "SET_BIN_MASK_LO",
            PM4_SET_BIN_MASK_HI => "SET_BIN_MASK_HI",
            PM4_SET_BIN_SELECT_LO => "SET_BIN_SELECT_LO",
            PM4_SET_BIN_SELECT_HI => "SET_BIN_SELECT_HI",
            _ => "UNKNOWN",
        }
    }
}

// =========================================================================
// Packet Builders
// =========================================================================

/// Builds a PM4 Type 0 Packet (Writes consecutive registers), MakePacketType0
pub fn build_type0(base_reg: u32, values: &[u32]) -> Vec<u32> {
    assert!(!values.is_empty(), "Values must not be empty");
    let count = (values.len() - 1) as u32;
    let header = (PM4_TYPE_0 << 30) | ((count & 0x3FFF) << 16) | (base_reg & 0x7FFF);
    let mut pkt = Vec::with_capacity(values.len() + 1);
    pkt.push(header);
    pkt.extend_from_slice(values);
    pkt
}

/// Builds a PM4 Type 3 Packet (Opcode + payload), MakePacketType3 without predicate
pub fn build_type3(opcode: u8, data: &[u32]) -> Vec<u32> {
    assert!(!data.is_empty(), "Data must not be empty");
    let count = (data.len() - 1) as u32;
    let header = (PM4_TYPE_3 << 30) | ((count & 0x3FFF) << 16) | (((opcode & 0x7F) as u32) << 8);
    let mut pkt = Vec::with_capacity(data.len() + 1);
    pkt.push(header);
    pkt.extend_from_slice(data);
    pkt
}

/// Writes one register (a Type-0 packet).
pub fn build_set_context_reg(reg: u32, value: u32) -> Vec<u32> {
    build_type0(reg, &[value])
}

/// SET_CONSTANT (type 0 = ALU) of float4 constants from c`start_idx` (ALU registers 0x4000 + 4 * index).
pub fn build_load_alu_constant(start_idx: u32, vectors: &[[f32; 4]]) -> Vec<u32> {
    let mut payload = Vec::with_capacity(1 + vectors.len() * 4);
    payload.push((start_idx * 4) & 0x7FF);
    for v in vectors {
        payload.extend(v.iter().map(|f| f.to_bits()));
    }
    build_type3(PM4_SET_CONSTANT, &payload)
}

/// Helper to build PM4_DRAW_INDX (Opcode 0x22). Payload layout of the old software path, not the Xenos one.
pub fn build_draw_indexed(prim_type: u32, num_indices: u32, index_ptr: u32) -> Vec<u32> {
    build_type3(PM4_DRAW_INDX, &[prim_type, num_indices, index_ptr, 0x0002])
}

/// SET_CONSTANT (type 0 = ALU) of raw values from ALU constant c`reg_idx`.
pub fn build_set_constant(reg_idx: u32, values: &[u32]) -> Vec<u32> {
    let mut payload = Vec::with_capacity(1 + values.len());
    payload.push((reg_idx * 4) & 0x7FF);
    payload.extend_from_slice(values);
    build_type3(PM4_SET_CONSTANT, &payload)
}

/// Helper to bind a Vertex Shader or Pixel Shader program address
pub fn build_set_shader_program(is_vertex_shader: bool, program_addr: u32) -> Vec<u32> {
    let reg = if is_vertex_shader { SQ_VS_PROGRAM_ADDR } else { SQ_PS_PROGRAM_ADDR };
    build_set_context_reg(reg, program_addr)
}

/// Helper to build PM4_WAIT_FOR_IDLE (Opcode 0x26)
pub fn build_wait_for_idle() -> Vec<u32> {
    build_type3(PM4_WAIT_FOR_IDLE, &[0x00000000])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type3_header_uses_7_bit_opcode_and_predicate_bit() {
        // INTERRUPT, 1 dword, predicated; bit 7 of the opcode field is not part of the opcode
        let (p, n) = Pm4Packet::decode(&[0xC000_D401, 0x1]).unwrap();
        assert_eq!(n, 2);
        match p {
            Pm4Packet::Type3 { opcode, predicated, data } => assert_eq!((opcode, predicated, data), (PM4_INTERRUPT, true, vec![1])),
            _ => panic!("not type 3"),
        }
        assert_eq!(Pm4Packet::opcode_name(PM4_INTERRUPT), "INTERRUPT");
        assert_eq!(build_type3(PM4_XE_SWAP, &[1, 2]), vec![0xC001_6400, 1, 2]);
    }

    #[test]
    fn type0_one_register_flag() {
        let (p, n) = Pm4Packet::decode(&[0x0001_8123, 7, 8]).unwrap();
        assert_eq!(n, 3);
        match p {
            Pm4Packet::Type0 { base_reg, one_reg, values } => assert_eq!((base_reg, one_reg, values), (0x0123, true, vec![7, 8])),
            _ => panic!("not type 0"),
        }
        assert_eq!(build_type0(0x2000, &[1, 2, 3]), vec![0x0002_2000, 1, 2, 3]);
        assert!(Pm4Packet::decode(&[0x0002_2000, 1]).is_err(), "incomplete");
    }

    #[test]
    fn type1_and_type2() {
        let (p, n) = Pm4Packet::decode(&[0x4000_0000 | (0x12 << 11) | 0x34, 5, 6]).unwrap();
        assert_eq!(n, 3);
        assert!(matches!(p, Pm4Packet::Type1 { reg1: 0x34, val1: 5, reg2: 0x12, val2: 6 }));
        assert!(matches!(Pm4Packet::decode(&[0x8000_0000]).unwrap(), (Pm4Packet::Type2Nop, 1)));
    }
}
