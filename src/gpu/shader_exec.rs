// Register indices from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik. See docs/THIRD_PARTY.md. Xenia Canary
// commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/register_table.inc                 PA_CL_VTE_CNTL, PA_CL_VPORT_*, VGT_INDX_OFFSET
//   src/xenia/gpu/registers.h                        PA_CL_VTE_CNTL fields, VGT_DMA_SIZE (num_words, swap_mode)
//   src/xenia/gpu/pm4_command_processor_implement.h  draw index sources (DMA base/size, auto index)
//
//! XENON_SHADER_EXEC=<n>: at each of the first n draws, run the active vertex shader (gpu/shader_interp.rs) on the
//! draw's first vertices with the real constants and vertex data, print clip and screen positions and
//! interpolators, then run the pixel shader on the first vertex's interpolators (textures: stub sampler). Diagnostics
//! only: the results are not drawn.

use super::draw_log::DrawInitiator;
use super::pm4::{PM4_DRAW_INDX, PM4_DRAW_INDX_BIN};
use super::shader_env::GpuShaderEnv;
use super::shader_interp::{run, ShaderState, Vec4};
use super::shader_log::shader_hash;
use super::texture::GpuTextureSampler;
use super::ucode::ShaderType;
use super::xenos::guest_endian_swap;
use crate::memory::MemoryBus;

const PA_CL_VTE_CNTL: u32 = 0x2206;
const PA_CL_VPORT_XSCALE: u32 = 0x210F;
const PA_CL_VPORT_XOFFSET: u32 = 0x2110;
const PA_CL_VPORT_YSCALE: u32 = 0x2111;
const PA_CL_VPORT_YOFFSET: u32 = 0x2112;
const VGT_INDX_OFFSET: u32 = 0x2102;

/// Vertices run per draw.
const VERTICES_PER_DRAW: u32 = 4;

pub struct ShaderExec {
    remaining: u32,
    draws: u64,
}

fn fmt4(v: &Vec4) -> String {
    format!("({:.4}, {:.4}, {:.4}, {:.4})", v[0], v[1], v[2], v[3])
}

/// Screen position of a clip-space position through PA_CL_VTE_CNTL and the viewport (x/y scale and offset enables
/// in bits 0..3, VTX_XY_FMT in bit 8: xy already divided by w).
pub fn screen_position(bus: &MemoryBus, p: &Vec4) -> (f32, f32) {
    let r = |i| bus.gpu_regs.get(i);
    let f = |i| f32::from_bits(r(i));
    let vte = r(PA_CL_VTE_CNTL);
    let (x, y) = if vte & (1 << 8) != 0 { (p[0], p[1]) } else { (p[0] / p[3], p[1] / p[3]) };
    let x = x * if vte & 1 != 0 { f(PA_CL_VPORT_XSCALE) } else { 1.0 } + if vte & 2 != 0 { f(PA_CL_VPORT_XOFFSET) } else { 0.0 };
    let y = y * if vte & 4 != 0 { f(PA_CL_VPORT_YSCALE) } else { 1.0 } + if vte & 8 != 0 { f(PA_CL_VPORT_YOFFSET) } else { 0.0 };
    (x, y)
}

/// The draw's vertex indices (at most `limit`), VGT_INDX_OFFSET applied, or why they cannot be read.
pub fn draw_indices(bus: &MemoryBus, opcode: u8, data: &[u32], limit: u32) -> Result<Vec<u32>, String> {
    let dma = opcode == PM4_DRAW_INDX || opcode == PM4_DRAW_INDX_BIN;
    let di = DrawInitiator::decode(data.get(if dma { 1 } else { 0 }).copied().unwrap_or(0));
    let count = di.num_indices.min(limit);
    let offset = bus.gpu_regs.get(VGT_INDX_OFFSET);
    let raw: Vec<u32> = match di.source_select {
        // auto index
        2 => (0..count).collect(),
        // DMA: VGT_DMA_BASE, VGT_DMA_SIZE (num_words in bits 0..23, swap mode in 30..31)
        0 => {
            let (base, size) = match (data.get(2), data.get(3)) {
                (Some(&b), Some(&s)) if dma => (b & 0x1FFF_FFFF, s),
                _ => return Err("DMA indices without VGT_DMA_BASE/SIZE".into()),
            };
            let swap = size >> 30;
            (0..count)
                .map(|i| {
                    if di.index_32bit {
                        guest_endian_swap(bus.read_u32((base & !3) as u64 + 4 * i as u64).unwrap_or(0), swap)
                    } else {
                        // 16-bit: k8in16 reads as the guest wrote it, kNone byte-swapped
                        let v = bus.read_u16((base & !1) as u64 + 2 * i as u64).unwrap_or(0);
                        (if swap == 0 { v.swap_bytes() } else { v }) as u32
                    }
                })
                .collect()
        }
        _ => return Err("immediate indices are not handled".into()),
    };
    Ok(raw.into_iter().map(|i| i.wrapping_add(offset) & 0x00FF_FFFF).collect())
}

impl ShaderExec {
    pub fn new(draws: u32) -> Self {
        Self { remaining: draws, draws: 0 }
    }

    pub fn draw(&mut self, bus: &MemoryBus, opcode: u8, data: &[u32], vertex_shader: &[u32], pixel_shader: &[u32]) {
        self.draws += 1;
        if self.remaining == 0 {
            return;
        }
        self.remaining -= 1;
        let dma = opcode == PM4_DRAW_INDX || opcode == PM4_DRAW_INDX_BIN;
        let di = DrawInitiator::decode(data.get(if dma { 1 } else { 0 }).copied().unwrap_or(0));
        println!("  [SHADER EXEC] draw #{} prim={} n={} vs {:016x} ps {:016x}", self.draws, di.prim_type, di.num_indices,
            shader_hash(vertex_shader), shader_hash(pixel_shader));
        let indices = match draw_indices(bus, opcode, data, VERTICES_PER_DRAW) {
            Ok(i) => i,
            Err(e) => {
                println!("    !! {}", e);
                return;
            }
        };
        let mut sampler = GpuTextureSampler::recording(bus);
        let mut first: Option<ShaderState> = None;
        for &index in &indices {
            let mut state = ShaderState::default();
            state.r[0][0] = index as f32;
            let mut env = GpuShaderEnv { bus, sampler: &mut sampler };
            if let Err(e) = run(vertex_shader, ShaderType::Vertex, &mut state, &mut env) {
                println!("    vertex {}: !! {}", index, e);
                continue;
            }
            match state.exports.position {
                Some(p) => {
                    let (sx, sy) = screen_position(bus, &p);
                    println!("    vertex {}: pos {} screen ({:.1}, {:.1})", index, fmt4(&p), sx, sy);
                }
                None => println!("    vertex {}: no position", index),
            }
            for (i, o) in state.exports.interpolators.iter().enumerate() {
                if let Some(v) = o {
                    println!("      o{} {}", i, fmt4(v));
                }
            }
            first.get_or_insert(state);
        }
        let Some(vs) = first else { return };
        // pixel shader inputs: the interpolators in r0, r1, ...
        let mut state = ShaderState::default();
        for (i, o) in vs.exports.interpolators.iter().enumerate() {
            if let Some(v) = o {
                state.r[i] = *v;
            }
        }
        let mut env = GpuShaderEnv { bus, sampler: &mut sampler };
        match run(pixel_shader, ShaderType::Pixel, &mut state, &mut env) {
            Ok(()) => {
                let colors: Vec<String> = state.exports.colors.iter().enumerate().filter_map(|(i, c)| c.map(|c| format!("oC{} {}", i, fmt4(&c)))).collect();
                println!("    pixel (vertex {} inputs): {}{}", indices[0], colors.join(" "), if state.killed { " killed" } else { "" });
            }
            Err(e) => println!("    pixel: !! {}", e),
        }
        for (c, coords, result) in &sampler.fetches {
            let (w, h) = c.size_2d();
            println!("      tfetch type={} format={} {}x{} base={:08X} tiled={} pitch={} at ({:.4}, {:.4}) -> {}", c.fetch_type(), c.format(), w, h,
                c.base_address(), c.tiled(), c.pitch(), coords[0], coords[1], fmt4(result));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::pm4::PM4_DRAW_INDX_2;

    #[test]
    fn draw_indices_from_auto_and_dma_sources() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(VGT_INDX_OFFSET, 10);
        // DRAW_INDX_2, auto index, 3 indices
        assert_eq!(draw_indices(&bus, PM4_DRAW_INDX_2, &[0x0003_0088], 8).unwrap(), [10, 11, 12]);
        // DRAW_INDX, DMA 16-bit, k8in16: [viz query, initiator, base, size]
        for (i, v) in [5u16, 7, 9].iter().enumerate() {
            bus.write_u16(0x0020_0000 + 2 * i as u64, *v).unwrap();
        }
        let initiator = (3 << 16) | (0 << 6) | 4;
        assert_eq!(draw_indices(&bus, PM4_DRAW_INDX, &[0, initiator, 0x0020_0000, (1 << 30) | 2], 2).unwrap(), [15, 17]);
        // 32-bit, k8in32
        bus.write_u32(0x0030_0000, 0x0001_0000).unwrap();
        assert_eq!(draw_indices(&bus, PM4_DRAW_INDX, &[0, (1 << 16) | (1 << 11), 0x0030_0000, 2 << 30], 4).unwrap(), [0x0001_000A]);
        assert!(draw_indices(&bus, PM4_DRAW_INDX_2, &[(1 << 16) | (1 << 6)], 4).is_err(), "immediate");
    }

    #[test]
    fn screen_position_follows_the_viewport_enables() {
        let mut bus = MemoryBus::new();
        let p = [0.5, -0.5, 0.0, 2.0];
        assert_eq!(screen_position(&bus, &p), (0.25, -0.25), "no viewport: divided by w");
        bus.gpu_regs.set(PA_CL_VTE_CNTL, 0b1111);
        bus.gpu_regs.set(PA_CL_VPORT_XSCALE, 640f32.to_bits());
        bus.gpu_regs.set(PA_CL_VPORT_XOFFSET, 640f32.to_bits());
        bus.gpu_regs.set(PA_CL_VPORT_YSCALE, (-360f32).to_bits());
        bus.gpu_regs.set(PA_CL_VPORT_YOFFSET, 360f32.to_bits());
        assert_eq!(screen_position(&bus, &p), (800.0, 450.0));
        bus.gpu_regs.set(PA_CL_VTE_CNTL, 0b1111 | (1 << 8));
        assert_eq!(screen_position(&bus, &p), (960.0, 540.0), "VTX_XY_FMT: not divided");
    }
}
