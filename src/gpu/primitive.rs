// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/d3d12/pipeline_cache.cc  PipelineGeometryShader::kRectangleList (longest edge is the diagonal; the
//                                          fourth vertex is -v0 + v1 + v2 for the vertex opposite it)
//   src/xenia/gpu/xenos.h                  PrimitiveType enumeration (kPointList, kTriangleList, kRectangleList, etc.)
//   src/xenia/gpu/register_table.inc       PA_CL_VTE_CNTL, PA_CL_VPORT_*, PA_CL_CLIP_CNTL
//   src/xenia/gpu/spirv_shader_translator.cc  position epilogue: W from 1/W unless VTX_W0_FMT, XY/Z multiplied back
//                                          by W when VTX_XY_FMT / VTX_Z_FMT
//   src/xenia/gpu/draw_util.cc             GetHostViewportInfo: viewport enables, half-pixel offset for
//                                          PA_SU_VTX_CNTL.pix_center = kD3DZero, -W..W depth unless dx_clip_space_def,
//                                          no clipping with clip_disable
//
//! Primitive assembly, clipping and the viewport transform of the Xenos GPU (Phase 4.5a).
//!
//! Deviation (documented): with clipping enabled, triangles are clipped only against the near/far planes and W > 0;
//! X/Y are not clipped (the console's guard band) and the scissor limits the rasterized area.

use crate::gpu::shader_interp::Vec4;
use crate::memory::MemoryBus;

pub const PA_CL_VTE_CNTL: u32 = 0x2206;
pub const PA_CL_VPORT_XSCALE: u32 = 0x210F;
pub const PA_CL_VPORT_XOFFSET: u32 = 0x2110;
pub const PA_CL_VPORT_YSCALE: u32 = 0x2111;
pub const PA_CL_VPORT_YOFFSET: u32 = 0x2112;
pub const PA_CL_VPORT_ZSCALE: u32 = 0x2113;
pub const PA_CL_VPORT_ZOFFSET: u32 = 0x2114;
pub const PA_CL_CLIP_CNTL: u32 = 0x2204;
const PA_SU_SC_MODE_CNTL: u32 = 0x2205;
const PA_SC_WINDOW_OFFSET: u32 = 0x2080;
const PA_SU_VTX_CNTL: u32 = 0x2302;

/// A vertex shader output: clip-space position (after the VTX_*_FMT corrections) and interpolators.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    pub position: Vec4,
    pub interpolators: [Option<Vec4>; 16],
}

impl Default for Vertex {
    fn default() -> Self {
        Self { position: [0.0, 0.0, 0.0, 1.0], interpolators: [None; 16] }
    }
}

impl Vertex {
    /// `a + t * (b - a)` for the position and the interpolators.
    fn lerp(a: &Vertex, b: &Vertex, t: f32) -> Vertex {
        let mix = |p: &Vec4, q: &Vec4| -> Vec4 { std::array::from_fn(|i| p[i] + t * (q[i] - p[i])) };
        let mut out = Vertex { position: mix(&a.position, &b.position), interpolators: [None; 16] };
        for i in 0..16 {
            out.interpolators[i] = match (a.interpolators[i], b.interpolators[i]) {
                (Some(p), Some(q)) => Some(mix(&p, &q)),
                (p, q) => p.or(q),
            };
        }
        out
    }
}

/// The exported position as clip space (Xenia's epilogue): W = 1/W unless VTX_W0_FMT (bit 10); XY and Z were
/// divided by W when VTX_XY_FMT (bit 8) / VTX_Z_FMT (bit 9).
pub fn clip_position(bus: &MemoryBus, p: Vec4) -> Vec4 {
    let vte = bus.gpu_regs.get(PA_CL_VTE_CNTL);
    let w = if vte & (1 << 10) != 0 { p[3] } else { 1.0 / p[3] };
    let xy = if vte & (1 << 8) != 0 { w } else { 1.0 };
    let z = if vte & (1 << 9) != 0 { w } else { 1.0 };
    [p[0] * xy, p[1] * xy, p[2] * z, w]
}

/// A vertex after the viewport transform.
#[derive(Debug, Clone, PartialEq)]
pub struct ScreenVertex {
    /// Pixel coordinates, pixel centers at .5.
    pub x: f32,
    pub y: f32,
    /// Depth in [0, 1].
    pub z: f32,
    pub inv_w: f32,
    pub interpolators: [Option<Vec4>; 16],
}

/// Viewport transform of a clip-space vertex (W > 0).
pub fn project(bus: &MemoryBus, v: &Vertex) -> ScreenVertex {
    let r = |i| bus.gpu_regs.get(i);
    let f = |i| f32::from_bits(r(i));
    let vte = r(PA_CL_VTE_CNTL);
    let inv_w = 1.0 / v.position[3];
    let [x, y, z, _] = v.position.map(|c| c * inv_w);
    let z = if r(PA_CL_CLIP_CNTL) & (1 << 19) != 0 { z } else { z * 0.5 + 0.5 };
    let scale = |bit: u32, reg: u32| if vte & (1 << bit) != 0 { f(reg) } else { 1.0 };
    let offset = |bit: u32, reg: u32| if vte & (1 << bit) != 0 { f(reg) } else { 0.0 };
    let mut sx = x * scale(0, PA_CL_VPORT_XSCALE) + offset(1, PA_CL_VPORT_XOFFSET);
    let mut sy = y * scale(2, PA_CL_VPORT_YSCALE) + offset(3, PA_CL_VPORT_YOFFSET);
    let sz = z * scale(4, PA_CL_VPORT_ZSCALE) + offset(5, PA_CL_VPORT_ZOFFSET);
    // D3D9 pixel centers are at integer coordinates
    if r(PA_SU_VTX_CNTL) & 1 == 0 {
        sx += 0.5;
        sy += 0.5;
    }
    if r(PA_SU_SC_MODE_CNTL) & (1 << 16) != 0 {
        let o = r(PA_SC_WINDOW_OFFSET);
        sx += (((o << 17) as i32) >> 17) as f32;
        sy += ((((o >> 16) << 17) as i32) >> 17) as f32;
    }
    ScreenVertex { x: sx, y: sy, z: sz.clamp(0.0, 1.0), inv_w, interpolators: v.interpolators }
}

/// Clips a triangle against the near/far planes and W > 0 (Sutherland-Hodgman); empty if nothing is left.
pub fn clip_triangle(bus: &MemoryBus, tri: &[Vertex; 3]) -> Vec<Vertex> {
    let clip = bus.gpu_regs.get(PA_CL_CLIP_CNTL);
    const W_MIN: f32 = 1e-6;
    let mut planes: Vec<fn(&Vec4) -> f32> = vec![|p| p[3] - W_MIN];
    if clip & (1 << 16) == 0 {
        planes.push(|p| p[3] - p[2]);
        planes.push(if clip & (1 << 19) != 0 { |p: &Vec4| p[2] } else { |p: &Vec4| p[2] + p[3] });
    }
    let mut poly: Vec<Vertex> = tri.to_vec();
    for plane in planes {
        if poly.is_empty() {
            break;
        }
        let mut out = Vec::with_capacity(poly.len() + 1);
        for i in 0..poly.len() {
            let (a, b) = (&poly[i], &poly[(i + 1) % poly.len()]);
            let (da, db) = (plane(&a.position), plane(&b.position));
            if da >= 0.0 {
                out.push(a.clone());
            }
            if (da >= 0.0) != (db >= 0.0) {
                out.push(Vertex::lerp(a, b, da / (da - db)));
            }
        }
        poly = out;
    }
    if poly.len() < 3 { Vec::new() } else { poly }
}

/// The fourth vertex of a rectangle-list primitive: `-a + b + c`, `a` being the vertex opposite the diagonal b-c.
pub fn mirror_vertex(a: &Vertex, b: &Vertex, c: &Vertex) -> Vertex {
    let m = |p: &Vec4, q: &Vec4, r: &Vec4| -> Vec4 { std::array::from_fn(|i| -p[i] + q[i] + r[i]) };
    let mut v = Vertex { position: m(&a.position, &b.position, &c.position), interpolators: [None; 16] };
    for i in 0..16 {
        if let (Some(p), Some(q), Some(r)) = (a.interpolators[i], b.interpolators[i], c.interpolators[i]) {
            v.interpolators[i] = Some(m(&p, &q, &r));
        }
    }
    v
}

/// Assembles the draw's vertices into triangles according to `prim_type` (xenos::PrimitiveType).
pub fn assemble_primitives(prim_type: u32, vertices: &[Vertex]) -> Vec<[Vertex; 3]> {
    let tri = |a: &Vertex, b: &Vertex, c: &Vertex| [a.clone(), b.clone(), c.clone()];
    let mut triangles = Vec::new();
    match prim_type {
        // kTriangleList
        0x04 => triangles.extend(vertices.chunks_exact(3).map(|c| tri(&c[0], &c[1], &c[2]))),
        // kTriangleFan
        0x05 => {
            for i in 1..vertices.len().saturating_sub(1) {
                triangles.push(tri(&vertices[0], &vertices[i], &vertices[i + 1]));
            }
        }
        // kTriangleStrip: odd triangles keep the winding
        0x06 => {
            for i in 0..vertices.len().saturating_sub(2) {
                let (a, b) = if i % 2 == 0 { (i, i + 1) } else { (i + 1, i) };
                triangles.push(tri(&vertices[a], &vertices[b], &vertices[i + 2]));
            }
        }
        // kRectangleList: the longest edge is the diagonal (Xenia's geometry shader strip: 012 / 120 / 201, then v3)
        0x08 => {
            for c in vertices.chunks_exact(3) {
                let len2 = |a: &Vertex, b: &Vertex| (a.position[0] - b.position[0]).powi(2) + (a.position[1] - b.position[1]).powi(2);
                let (e12, e20, e01) = (len2(&c[1], &c[2]), len2(&c[2], &c[0]), len2(&c[0], &c[1]));
                let (a, b, d) = if e12 > e20 && e12 > e01 {
                    (0, 1, 2)
                } else if e20 > e01 {
                    (1, 2, 0)
                } else {
                    (2, 0, 1)
                };
                let v3 = mirror_vertex(&c[a], &c[b], &c[d]);
                triangles.push(tri(&c[a], &c[b], &c[d]));
                triangles.push(tri(&c[b], &c[d], &v3));
            }
        }
        // kQuadList
        0x0D => {
            for c in vertices.chunks_exact(4) {
                triangles.push(tri(&c[0], &c[1], &c[2]));
                triangles.push(tri(&c[0], &c[2], &c[3]));
            }
        }
        _ => {}
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> Vertex {
        let mut v = Vertex { position: [x, y, 0.0, 1.0], ..Default::default() };
        v.interpolators[0] = Some([x, y, 0.0, 1.0]);
        v
    }

    #[test]
    fn rectangle_list_mirrors_the_vertex_opposite_the_longest_edge() {
        // 12 is the diagonal: v3 = -v0 + v1 + v2
        let tris = assemble_primitives(0x08, &[at(0.0, 0.0), at(4.0, 0.0), at(0.0, 2.0)]);
        assert_eq!(tris.len(), 2);
        assert_eq!(tris[1][2].position, [4.0, 2.0, 0.0, 1.0]);
        assert_eq!(tris[1][2].interpolators[0], Some([4.0, 2.0, 0.0, 1.0]));
        // the dash's resolve rectangles: (-0.5,-0.5), (1279.5,-0.5), (1279.5,719.5); 20 is the diagonal, v1 mirrored
        let tris = assemble_primitives(0x08, &[at(-0.5, -0.5), at(1279.5, -0.5), at(1279.5, 719.5)]);
        assert_eq!(tris[0][0].position[..2], [1279.5, -0.5]);
        assert_eq!(tris[1][2].position[..2], [-0.5, 719.5]);
    }

    #[test]
    fn triangle_strip_keeps_the_winding() {
        let v: Vec<Vertex> = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].iter().map(|&(x, y)| at(x, y)).collect();
        let tris = assemble_primitives(0x06, &v);
        assert_eq!(tris[1][0].position[..2], [0.0, 1.0]);
        assert_eq!(tris[1][1].position[..2], [1.0, 0.0]);
    }

    #[test]
    fn positions_follow_the_vte_formats_viewport_and_pixel_center() {
        let mut bus = MemoryBus::new();
        // the dash's rectangles: VTX_XY_FMT | VTX_Z_FMT, no viewport, W given as 1/W
        bus.gpu_regs.set(PA_CL_VTE_CNTL, 0x300);
        bus.gpu_regs.set(PA_SU_VTX_CNTL, 4);
        let p = clip_position(&bus, [-0.5, 719.5, 0.25, 2.0]);
        assert_eq!(p, [-0.25, 359.75, 0.125, 0.5]);
        let s = project(&bus, &Vertex { position: p, ..Default::default() });
        assert_eq!((s.x, s.y), (0.0, 720.0));
        assert_eq!(s.z, 0.25 * 0.5 + 0.5);
        // the dash's UI: viewport 640/-360 scale and offset, DX clip space
        bus.gpu_regs.set(PA_CL_VTE_CNTL, 0x43F);
        bus.gpu_regs.set(PA_CL_CLIP_CNTL, 1 << 19);
        bus.gpu_regs.set(PA_CL_VPORT_XSCALE, 640f32.to_bits());
        bus.gpu_regs.set(PA_CL_VPORT_XOFFSET, 640f32.to_bits());
        bus.gpu_regs.set(PA_CL_VPORT_YSCALE, (-360f32).to_bits());
        bus.gpu_regs.set(PA_CL_VPORT_YOFFSET, 360f32.to_bits());
        let s = project(&bus, &Vertex { position: clip_position(&bus, [1.0, 1.0, 0.0, 1.0]), ..Default::default() });
        assert_eq!((s.x, s.y, s.z), (1280.5, 0.5, 0.0));
    }

    #[test]
    fn clipping_cuts_at_the_near_plane() {
        let mut bus = MemoryBus::new();
        bus.gpu_regs.set(PA_CL_CLIP_CNTL, 1 << 19);
        let v = |z: f32| Vertex { position: [0.0, z, z, 1.0], ..Default::default() };
        let poly = clip_triangle(&bus, &[v(-1.0), v(0.5), Vertex { position: [1.0, 0.5, 0.5, 1.0], ..Default::default() }]);
        assert_eq!(poly.len(), 4);
        assert!(poly.iter().all(|p| p.position[2] >= 0.0));
        // clip_disable: only W > 0 matters
        bus.gpu_regs.set(PA_CL_CLIP_CNTL, 1 << 16);
        assert_eq!(clip_triangle(&bus, &[v(-1.0), v(0.5), v(2.0)]).len(), 3);
        let behind = Vertex { position: [0.0, 0.0, 0.0, -1.0], ..Default::default() };
        assert!(clip_triangle(&bus, &[behind.clone(), behind.clone(), behind]).is_empty());
    }
}
