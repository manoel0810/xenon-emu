// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/d3d12/pipeline_cache.cc  PipelineGeometryShader::kRectangleList (mirroring v3 = -v0 + v1 + v2)
//   src/xenia/gpu/xenos.h                  PrimitiveType enumeration (kPointList, kTriangleList, kRectangleList, etc.)
//   src/xenia/gpu/register_table.inc       PA_CL_VTE_CNTL, PA_CL_VPORT_*
//
//! Primitive assembly, viewport transformations, and primitive topologies for the Xenos GPU (Phase 4.5a).

use crate::gpu::shader_interp::Vec4;
use crate::memory::MemoryBus;

pub const PA_CL_VTE_CNTL: u32 = 0x2206;
pub const PA_CL_VPORT_XSCALE: u32 = 0x210F;
pub const PA_CL_VPORT_XOFFSET: u32 = 0x2110;
pub const PA_CL_VPORT_YSCALE: u32 = 0x2111;
pub const PA_CL_VPORT_YOFFSET: u32 = 0x2112;
pub const PA_CL_VPORT_ZSCALE: u32 = 0x2113;
pub const PA_CL_VPORT_ZOFFSET: u32 = 0x2114;

/// Transformed vertex ready for triangle assembly and rasterization.
#[derive(Debug, Clone, PartialEq)]
pub struct Vertex {
    /// Clip-space position (x, y, z, w).
    pub position: Vec4,
    /// Screen-space position (sx, sy) in pixels.
    pub screen: (f32, f32),
    /// Screen-space depth z in [0.0, 1.0].
    pub z: f32,
    /// Reciprocal of clip-space w (1.0 / w) used for perspective-correct interpolation.
    pub inv_w: f32,
    /// Vertex shader export interpolators (o0..o15).
    pub interpolators: [Option<Vec4>; 16],
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0, 1.0],
            screen: (0.0, 0.0),
            z: 0.0,
            inv_w: 1.0,
            interpolators: [None; 16],
        }
    }
}

/// Screen position of a clip-space position through PA_CL_VTE_CNTL and the viewport.
/// x/y scale and offset enables in bits 0..3, VTX_XY_FMT in bit 8: if set, xy are already divided by w.
pub fn screen_position(bus: &MemoryBus, p: &Vec4) -> (f32, f32) {
    let r = |i| bus.gpu_regs.get(i);
    let f = |i| f32::from_bits(r(i));
    let vte = r(PA_CL_VTE_CNTL);
    let (x, y) = if vte & (1 << 8) != 0 {
        (p[0], p[1])
    } else if p[3].abs() > 1e-6 {
        (p[0] / p[3], p[1] / p[3])
    } else {
        (p[0], p[1])
    };
    let x = x * if vte & 1 != 0 { f(PA_CL_VPORT_XSCALE) } else { 1.0 }
        + if vte & 2 != 0 { f(PA_CL_VPORT_XOFFSET) } else { 0.0 };
    let y = y * if vte & 4 != 0 { f(PA_CL_VPORT_YSCALE) } else { 1.0 }
        + if vte & 8 != 0 { f(PA_CL_VPORT_YOFFSET) } else { 0.0 };
    (x, y)
}

/// Screen depth of a clip-space position through PA_CL_VTE_CNTL and the viewport.
/// z scale and offset enables in bits 4..5, VTX_Z_FMT in bit 9: if set, z is already divided by w.
pub fn screen_depth(bus: &MemoryBus, p: &Vec4) -> f32 {
    let r = |i| bus.gpu_regs.get(i);
    let f = |i| f32::from_bits(r(i));
    let vte = r(PA_CL_VTE_CNTL);
    let z_norm = if vte & (1 << 9) != 0 {
        p[2]
    } else if p[3].abs() > 1e-6 {
        p[2] / p[3]
    } else {
        p[2]
    };
    let z = z_norm * if vte & (1 << 4) != 0 { f(PA_CL_VPORT_ZSCALE) } else { 1.0 }
        + if vte & (1 << 5) != 0 { f(PA_CL_VPORT_ZOFFSET) } else { 0.0 };
    z.clamp(0.0, 1.0)
}

/// Triangle formed by 3 vertices.
#[derive(Debug, Clone, PartialEq)]
pub struct Triangle {
    pub v: [Vertex; 3],
}

/// Mirrors vertex across diagonal to form the fourth vertex of a rectangle:
/// v3 = v1 + v2 - v0 (Xenia Canary pipeline_cache.cc: kRectangleList).
pub fn mirror_vertex(v0: &Vertex, v1: &Vertex, v2: &Vertex) -> Vertex {
    let mut v3 = Vertex {
        position: [
            v1.position[0] + v2.position[0] - v0.position[0],
            v1.position[1] + v2.position[1] - v0.position[1],
            v1.position[2] + v2.position[2] - v0.position[2],
            v1.position[3] + v2.position[3] - v0.position[3],
        ],
        screen: (
            v1.screen.0 + v2.screen.0 - v0.screen.0,
            v1.screen.1 + v2.screen.1 - v0.screen.1,
        ),
        z: (v1.z + v2.z - v0.z).clamp(0.0, 1.0),
        inv_w: v1.inv_w + v2.inv_w - v0.inv_w,
        interpolators: [None; 16],
    };
    for i in 0..16 {
        match (v0.interpolators[i], v1.interpolators[i], v2.interpolators[i]) {
            (Some(i0), Some(i1), Some(i2)) => {
                v3.interpolators[i] = Some([
                    i1[0] + i2[0] - i0[0],
                    i1[1] + i2[1] - i0[1],
                    i1[2] + i2[2] - i0[2],
                    i1[3] + i2[3] - i0[3],
                ]);
            }
            (_, Some(i1), _) => v3.interpolators[i] = Some(i1),
            (_, _, Some(i2)) => v3.interpolators[i] = Some(i2),
            _ => {}
        }
    }
    v3
}

/// Assembles a slice of processed vertices into triangles according to `prim_type`.
pub fn assemble_primitives(prim_type: u32, vertices: &[Vertex]) -> Vec<Triangle> {
    let mut triangles = Vec::new();
    match prim_type {
        // 0x01: Point list (1 vertex -> 1x1 screen quad)
        0x01 => {
            for v in vertices {
                let (sx, sy) = v.screen;
                let mut v0 = v.clone();
                let mut v1 = v.clone();
                let mut v2 = v.clone();
                let mut v3 = v.clone();
                v0.screen = (sx - 0.5, sy - 0.5);
                v1.screen = (sx + 0.5, sy - 0.5);
                v2.screen = (sx - 0.5, sy + 0.5);
                v3.screen = (sx + 0.5, sy + 0.5);
                triangles.push(Triangle { v: [v0, v1.clone(), v2.clone()] });
                triangles.push(Triangle { v: [v1, v3, v2] });
            }
        }
        // 0x04: Triangle list (groups of 3)
        0x04 => {
            for chunk in vertices.chunks_exact(3) {
                triangles.push(Triangle {
                    v: [chunk[0].clone(), chunk[1].clone(), chunk[2].clone()],
                });
            }
        }
        // 0x05: Triangle fan
        0x05 => {
            if vertices.len() >= 3 {
                let v0 = &vertices[0];
                for i in 1..vertices.len() - 1 {
                    triangles.push(Triangle {
                        v: [v0.clone(), vertices[i].clone(), vertices[i + 1].clone()],
                    });
                }
            }
        }
        // 0x06: Triangle strip (alternating winding)
        0x06 => {
            if vertices.len() >= 3 {
                for i in 0..vertices.len() - 2 {
                    if i % 2 == 0 {
                        triangles.push(Triangle {
                            v: [vertices[i].clone(), vertices[i + 1].clone(), vertices[i + 2].clone()],
                        });
                    } else {
                        triangles.push(Triangle {
                            v: [vertices[i + 1].clone(), vertices[i].clone(), vertices[i + 2].clone()],
                        });
                    }
                }
            }
        }
        // 0x08: Rectangle list (3 vertices per rectangle decomposed into 2 triangles)
        0x08 => {
            for chunk in vertices.chunks_exact(3) {
                let (v0, v1, v2) = (&chunk[0], &chunk[1], &chunk[2]);
                let v3 = mirror_vertex(v0, v1, v2);
                triangles.push(Triangle {
                    v: [v0.clone(), v1.clone(), v2.clone()],
                });
                triangles.push(Triangle {
                    v: [v1.clone(), v3, v2.clone()],
                });
            }
        }
        // 0x0D: Quad list (groups of 4)
        0x0D => {
            for chunk in vertices.chunks_exact(4) {
                triangles.push(Triangle {
                    v: [chunk[0].clone(), chunk[1].clone(), chunk[2].clone()],
                });
                triangles.push(Triangle {
                    v: [chunk[0].clone(), chunk[2].clone(), chunk[3].clone()],
                });
            }
        }
        _ => {
            // Unsupported or fallback
        }
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectlist_generates_two_triangles_with_mirrored_vertex() {
        let mut v0 = Vertex::default();
        v0.screen = (100.0, 100.0);
        v0.interpolators[0] = Some([0.0, 0.0, 0.0, 1.0]);

        let mut v1 = Vertex::default();
        v1.screen = (200.0, 100.0);
        v1.interpolators[0] = Some([1.0, 0.0, 0.0, 1.0]);

        let mut v2 = Vertex::default();
        v2.screen = (100.0, 300.0);
        v2.interpolators[0] = Some([0.0, 1.0, 0.0, 1.0]);

        let tris = assemble_primitives(0x08, &[v0, v1, v2]);
        assert_eq!(tris.len(), 2);

        // First triangle: (v0, v1, v2)
        assert_eq!(tris[0].v[0].screen, (100.0, 100.0));
        assert_eq!(tris[0].v[1].screen, (200.0, 100.0));
        assert_eq!(tris[0].v[2].screen, (100.0, 300.0));

        // Second triangle: (v1, v3, v2) where v3 = v1 + v2 - v0 = (200, 300)
        assert_eq!(tris[1].v[0].screen, (200.0, 100.0));
        assert_eq!(tris[1].v[1].screen, (200.0, 300.0));
        assert_eq!(tris[1].v[2].screen, (100.0, 300.0));

        // Interpolator on v3: i1 + i2 - i0 = (1.0, 1.0, 0.0, 1.0)
        let interp_v3 = tris[1].v[1].interpolators[0].unwrap();
        assert_eq!(interp_v3, [1.0, 1.0, 0.0, 1.0]);
    }

    #[test]
    fn triangle_strip_preserves_consistent_winding() {
        let v = vec![
            Vertex { screen: (0.0, 0.0), ..Default::default() },
            Vertex { screen: (1.0, 0.0), ..Default::default() },
            Vertex { screen: (0.0, 1.0), ..Default::default() },
            Vertex { screen: (1.0, 1.0), ..Default::default() },
        ];
        let tris = assemble_primitives(0x06, &v);
        assert_eq!(tris.len(), 2);
        // First triangle: (0, 1, 2)
        assert_eq!(tris[0].v[0].screen, (0.0, 0.0));
        assert_eq!(tris[0].v[1].screen, (1.0, 0.0));
        assert_eq!(tris[0].v[2].screen, (0.0, 1.0));
        // Second triangle: alternating winding (v[2], v[1], v[3]) = ((0, 1), (1, 0), (1, 1))
        assert_eq!(tris[1].v[0].screen, (0.0, 1.0));
        assert_eq!(tris[1].v[1].screen, (1.0, 0.0));
        assert_eq!(tris[1].v[2].screen, (1.0, 1.0));
    }
}
