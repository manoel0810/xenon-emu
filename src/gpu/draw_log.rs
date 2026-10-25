// Register indices and names from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026
// Xenia Canary. See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/register_table.inc   register indices
//   src/xenia/gpu/registers.h          VGT_DRAW_INITIATOR fields
//
//! XENON_DRAW_LOG=1: at every draw packet, print the decoded draw initiator and the render-target, viewport, shader
//! and resolve (RB_COPY_*) registers it would use, plus each new vertex/pixel shader blob once. Diagnostics only:
//! nothing here changes emulation.

use std::collections::HashSet;

use crate::gpu::pm4::{Pm4Packet, PM4_DRAW_INDX, PM4_DRAW_INDX_BIN};
use crate::memory::MemoryBus;

/// Registers shown at each draw (register_table.inc).
const DRAW_REGISTERS: &[(u32, &str)] = &[
    (0x2208, "RB_MODECONTROL"),
    (0x2000, "RB_SURFACE_INFO"),
    (0x2001, "RB_COLOR_INFO"),
    (0x2003, "RB_COLOR1_INFO"),
    (0x2002, "RB_DEPTH_INFO"),
    (0x2104, "RB_COLOR_MASK"),
    (0x2200, "RB_DEPTHCONTROL"),
    (0x2201, "RB_BLENDCONTROL0"),
    (0x2202, "RB_COLORCONTROL"),
    (0x2080, "PA_SC_WINDOW_OFFSET"),
    (0x2081, "PA_SC_WINDOW_SCISSOR_TL"),
    (0x2082, "PA_SC_WINDOW_SCISSOR_BR"),
    (0x200E, "PA_SC_SCREEN_SCISSOR_TL"),
    (0x200F, "PA_SC_SCREEN_SCISSOR_BR"),
    (0x2206, "PA_CL_VTE_CNTL"),
    (0x210F, "PA_CL_VPORT_XSCALE"),
    (0x2110, "PA_CL_VPORT_XOFFSET"),
    (0x2111, "PA_CL_VPORT_YSCALE"),
    (0x2112, "PA_CL_VPORT_YOFFSET"),
    (0x2204, "PA_CL_CLIP_CNTL"),
    (0x2205, "PA_SU_SC_MODE_CNTL"),
    (0x2302, "PA_SU_VTX_CNTL"),
    (0x2180, "SQ_PROGRAM_CNTL"),
    (0x2181, "SQ_CONTEXT_MISC"),
    (0x2102, "VGT_INDX_OFFSET"),
    (0x2318, "RB_COPY_CONTROL"),
    (0x2319, "RB_COPY_DEST_BASE"),
    (0x231A, "RB_COPY_DEST_PITCH"),
    (0x231B, "RB_COPY_DEST_INFO"),
    (0x2320, "RB_COPY_FUNC"),
    (0x231D, "RB_DEPTH_CLEAR"),
    (0x231E, "RB_COLOR_CLEAR"),
    (0x231F, "RB_COLOR_CLEAR_LO"),
];

/// First vertex fetch constant (SHADER_CONSTANT_FETCH_00_0); 6 dwords per fetch constant.
const FETCH_CONSTANTS: u32 = 0x4800;

/// Fields of VGT_DRAW_INITIATOR (registers.h).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawInitiator {
    pub prim_type: u32,
    pub source_select: u32,
    pub major_mode: u32,
    pub index_32bit: bool,
    pub not_eop: bool,
    pub num_indices: u32,
}

impl DrawInitiator {
    pub fn decode(v: u32) -> Self {
        Self {
            prim_type: v & 0x3F,
            source_select: (v >> 6) & 3,
            major_mode: (v >> 8) & 3,
            index_32bit: (v >> 11) & 1 != 0,
            not_eop: (v >> 12) & 1 != 0,
            num_indices: v >> 16,
        }
    }
}

fn prim_name(p: u32) -> &'static str {
    match p {
        0x00 => "none",
        0x01 => "point_list",
        0x02 => "line_list",
        0x03 => "line_strip",
        0x04 => "triangle_list",
        0x05 => "triangle_fan",
        0x06 => "triangle_strip",
        0x07 => "triangle_with_wflags",
        0x08 => "rectangle_list",
        0x0C => "line_loop",
        0x0D => "quad_list",
        0x0E => "quad_strip",
        0x0F => "polygon",
        _ => "other",
    }
}

fn mode_name(m: u32) -> &'static str {
    match m & 7 {
        0 => "ignore",
        4 => "color_depth",
        5 => "depth",
        6 => "copy",
        _ => "?",
    }
}

/// 64-bit FNV-1a of a shader blob, to identify it across draws.
pub fn shader_hash(words: &[u32]) -> u64 {
    words.iter().flat_map(|w| w.to_be_bytes()).fold(0xCBF2_9CE4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01B3))
}

pub struct DrawLog {
    pub draws: u64,
    printed_shaders: HashSet<u64>,
}

impl DrawLog {
    pub fn new() -> Self {
        Self { draws: 0, printed_shaders: HashSet::new() }
    }

    /// Prints one draw packet with the state it would use.
    pub fn log(&mut self, bus: &MemoryBus, opcode: u8, predicated: bool, data: &[u32], vertex_shader: &[u32], pixel_shader: &[u32]) {
        self.draws += 1;
        // DRAW_INDX(_BIN): [viz_query, initiator, (index base, index size)]; DRAW_INDX_2(_BIN): [initiator, indices...]
        let dma = opcode == PM4_DRAW_INDX || opcode == PM4_DRAW_INDX_BIN;
        let initiator = data.get(if dma { 1 } else { 0 }).copied().unwrap_or(0);
        let di = DrawInitiator::decode(initiator);
        let regs = &bus.gpu_regs;
        let source = ["dma", "immediate", "auto", "?"][di.source_select as usize];
        println!("  [DRAW #{}] {}{} initiator={:08X} prim={} ({}) src={} n={} idx32={} mode={} ({})", self.draws,
            Pm4Packet::opcode_name(opcode), if predicated { " pred" } else { "" }, initiator, di.prim_type, prim_name(di.prim_type),
            source, di.num_indices, di.index_32bit, regs.get(0x2208) & 7, mode_name(regs.get(0x2208)));
        if dma && data.len() >= 4 {
            println!("    index base={:08X} size={:08X}", data[2], data[3]);
        } else if !dma && data.len() > 1 {
            let shown: Vec<String> = data[1..].iter().take(8).map(|d| format!("{:08X}", d)).collect();
            println!("    immediate indices [{}]{}", shown.join(" "), if data.len() > 9 { " ..." } else { "" });
        }
        for chunk in DRAW_REGISTERS.chunks(4) {
            let line: Vec<String> = chunk.iter().map(|&(r, n)| format!("{}={:08X}", n, regs.get(r))).collect();
            println!("    {}", line.join(" "));
        }
        // every other non-zero context register (0x2000..0x23FF), by index
        let others: Vec<String> = (0x2000..0x2400u32)
            .filter(|r| !DRAW_REGISTERS.iter().any(|&(d, _)| d == *r) && regs.get(*r) != 0)
            .map(|r| format!("{:04X}={:08X}", r, regs.get(r)))
            .collect();
        for chunk in others.chunks(8) {
            println!("    ctx {}", chunk.join(" "));
        }
        let fetch: Vec<String> = (0..6).map(|i| format!("{:08X}", regs.get(FETCH_CONSTANTS + i))).collect();
        println!("    FETCH_00 [{}]", fetch.join(" "));
        for (kind, words) in [("VS", vertex_shader), ("PS", pixel_shader)] {
            let hash = shader_hash(words);
            println!("    {} {:016X} ({} dwords)", kind, hash, words.len());
            if !words.is_empty() && self.printed_shaders.insert(hash) {
                for (i, row) in words.chunks(6).enumerate() {
                    let row: Vec<String> = row.iter().map(|w| format!("{:08X}", w)).collect();
                    println!("      {} {:016X} +{:03}: {}", kind, hash, i * 6, row.join(" "));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_the_dash_draw_initiators() {
        // steady-state clear / resolve: 3-vertex auto-indexed rectangle list
        assert_eq!(DrawInitiator::decode(0x0003_0088), DrawInitiator { prim_type: 8, source_select: 2, major_mode: 0, index_32bit: false, not_eop: false, num_indices: 3 });
        // device init: 1-vertex auto-indexed point list
        let d = DrawInitiator::decode(0x0001_0081);
        assert_eq!((d.prim_type, d.source_select, d.num_indices), (1, 2, 1));
        assert_eq!(prim_name(8), "rectangle_list");
        assert_eq!(shader_hash(&[]), 0xCBF2_9CE4_8422_2325);
        assert_ne!(shader_hash(&[1]), shader_hash(&[2]));
    }
}
