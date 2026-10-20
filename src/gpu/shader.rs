use crate::gpu::texture::Texture;

/// ATI Xenos (Xbox 360) ALU Microcode Opcodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AluOpcode {
    Add,
    Mul,
    Mad,  // Multiply-Add: src0 * src1 + src2
    Dot3, // 3-component dot product
    Dot4, // 4-component dot product
    Mov,
    Min,
    Max,
    Slt,  // Set-on-less-than
    Sge,  // Set-on-greater-or-equal
}

/// ATI Xenos Texture and Vertex Fetch Opcodes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOpcode {
    VtxFetch,
    TexFetch,
}

/// Register types available to Xenos unified shader cores
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderReg {
    Temp(u8),       // r0..r31 (Temporary 128-bit vector registers)
    Constant(u8),   // c0..c255 (ALU constant vector registers)
    Attribute(u8),  // v0..v15 (Vertex input stream attributes)
    Export(u8),     // o0..o15 (Vertex exports: 0=Position, 1=Color0, 2=Tex0)
    PixelOut(u8),   // oC0..oC3 (Pixel shader render targets)
}

/// Component swizzle (X=0, Y=1, Z=2, W=3)
#[derive(Debug, Clone, Copy)]
pub struct Swizzle {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    pub w: u8,
}

impl Default for Swizzle {
    fn default() -> Self {
        Self { x: 0, y: 1, z: 2, w: 3 } // .xyzw (identity)
    }
}

/// Single decoded ATI Xenos ALU Vector Instruction
#[derive(Debug, Clone)]
pub struct AluInstruction {
    pub opcode: AluOpcode,
    pub dest: ShaderReg,
    pub write_mask: [bool; 4], // [x, y, z, w]
    pub src0: ShaderReg,
    pub swizzle0: Swizzle,
    pub src1: ShaderReg,
    pub swizzle1: Swizzle,
    pub src2: Option<ShaderReg>,
    pub swizzle2: Swizzle,
}

/// Input vertex data passed to the vertex shader
#[derive(Debug, Clone, Copy)]
pub struct VertexInput {
    pub position: [f32; 4], // (x, y, z, w)
    pub color: [f32; 4],    // (r, g, b, a) in 0.0..1.0
    pub uv: [f32; 2],       // (u, v) texture coordinates
    pub normal: [f32; 3],   // (nx, ny, nz)
}

/// Output vertex data emitted by the vertex shader
#[derive(Debug, Clone, Copy)]
pub struct VertexOutput {
    pub clip_pos: [f32; 4], // Homogeneous clip coordinates
    pub screen_x: f32,      // Projected pixel coordinate X (0..1280)
    pub screen_y: f32,      // Projected pixel coordinate Y (0..720)
    pub color: [f32; 4],    // Interpolated RGBA
    pub uv: [f32; 2],       // Interpolated UV
}

/// ATI Xenos Microcode Vertex Shader Program
#[derive(Debug, Clone)]
pub struct VertexShader {
    pub name: String,
    pub instructions: Vec<AluInstruction>,
    pub raw_bytecode: Vec<u32>,
}

impl VertexShader {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            instructions: Vec::new(),
            raw_bytecode: Vec::new(),
        }
    }

    /// Decodes Xenos microcode instructions from raw dword bytecode
    pub fn decode_bytecode(name: &str, words: &[u32]) -> Self {
        let mut vs = Self::new(name);
        vs.raw_bytecode = words.to_vec();

        // Xenos microcode words are structured as pairs of 32-bit dwords (64-bit instructions)
        for chunk in words.chunks_exact(2) {
            let dword0 = chunk[0];
            let dword1 = chunk[1];

            let op_code_raw = (dword0 >> 24) & 0x3F;
            let opcode = match op_code_raw {
                0x01 => AluOpcode::Add,
                0x02 => AluOpcode::Mul,
                0x03 => AluOpcode::Mad,
                0x04 => AluOpcode::Dot3,
                0x05 => AluOpcode::Dot4,
                0x06 => AluOpcode::Min,
                0x07 => AluOpcode::Max,
                0x08 => AluOpcode::Slt,
                0x09 => AluOpcode::Sge,
                _ => AluOpcode::Mov,
            };

            let dest_reg = ((dword0 >> 16) & 0x1F) as u8;
            let is_export = (dword0 & (1 << 23)) != 0;
            let dest = if is_export {
                ShaderReg::Export(dest_reg)
            } else {
                ShaderReg::Temp(dest_reg)
            };

            let src0_reg = (dword0 & 0xFF) as u8;
            let src0_is_const = (dword0 & (1 << 15)) != 0;
            let src0 = if src0_is_const {
                ShaderReg::Constant(src0_reg)
            } else {
                ShaderReg::Attribute(src0_reg)
            };

            let src1_reg = (dword1 & 0xFF) as u8;
            let src1_is_const = (dword1 & (1 << 15)) != 0;
            let src1 = if src1_is_const {
                ShaderReg::Constant(src1_reg)
            } else {
                ShaderReg::Temp(src1_reg)
            };

            vs.instructions.push(AluInstruction {
                opcode,
                dest,
                write_mask: [true, true, true, true],
                src0,
                swizzle0: Swizzle::default(),
                src1,
                swizzle1: Swizzle::default(),
                src2: None,
                swizzle2: Swizzle::default(),
            });
        }

        vs
    }

    /// Executes the Vertex Shader program on a single input vertex
    pub fn execute(
        &self,
        v_in: &VertexInput,
        constants: &[[f32; 4]; 256],
        vport_xscale: f32,
        vport_yscale: f32,
        vport_xoffset: f32,
        vport_yoffset: f32,
    ) -> VertexOutput {
        // Register state
        let mut r = [[0.0f32; 4]; 32];
        let mut exports = [[0.0f32; 4]; 16];

        // Seed attributes into r0..r3
        r[0] = v_in.position;
        r[1] = v_in.color;
        r[2] = [v_in.uv[0], v_in.uv[1], 0.0, 1.0];
        r[3] = [v_in.normal[0], v_in.normal[1], v_in.normal[2], 0.0];

        // Default transform: MVP matrix in constants c0..c3
        let mvp_0 = constants[0];
        let mvp_1 = constants[1];
        let mvp_2 = constants[2];
        let mvp_3 = constants[3];

        let has_custom_matrix = (mvp_0[0].abs() + mvp_1[1].abs() + mvp_2[2].abs() + mvp_3[3].abs()) > 0.01;

        let mut clip_pos = if has_custom_matrix {
            let px = v_in.position[0];
            let py = v_in.position[1];
            let pz = v_in.position[2];
            let pw = v_in.position[3];

            [
                px * mvp_0[0] + py * mvp_1[0] + pz * mvp_2[0] + pw * mvp_3[0],
                px * mvp_0[1] + py * mvp_1[1] + pz * mvp_2[1] + pw * mvp_3[1],
                px * mvp_0[2] + py * mvp_1[2] + pz * mvp_2[2] + pw * mvp_3[2],
                px * mvp_0[3] + py * mvp_1[3] + pz * mvp_2[3] + pw * mvp_3[3],
            ]
        } else {
            v_in.position
        };

        // Execute microcode ALU instructions if any exist
        for insn in &self.instructions {
            let val0 = self.read_reg(insn.src0, &r, constants, v_in);
            let val1 = self.read_reg(insn.src1, &r, constants, v_in);
            let val2 = insn.src2.map(|s| self.read_reg(s, &r, constants, v_in)).unwrap_or([0.0; 4]);

            let mut out = [0.0f32; 4];
            match insn.opcode {
                AluOpcode::Add => {
                    for i in 0..4 { out[i] = val0[i] + val1[i]; }
                }
                AluOpcode::Mul => {
                    for i in 0..4 { out[i] = val0[i] * val1[i]; }
                }
                AluOpcode::Mad => {
                    for i in 0..4 { out[i] = val0[i] * val1[i] + val2[i]; }
                }
                AluOpcode::Dot3 => {
                    let dot = val0[0] * val1[0] + val0[1] * val1[1] + val0[2] * val1[2];
                    out = [dot, dot, dot, dot];
                }
                AluOpcode::Dot4 => {
                    let dot = val0[0] * val1[0] + val0[1] * val1[1] + val0[2] * val1[2] + val0[3] * val1[3];
                    out = [dot, dot, dot, dot];
                }
                AluOpcode::Min => {
                    for i in 0..4 { out[i] = val0[i].min(val1[i]); }
                }
                AluOpcode::Max => {
                    for i in 0..4 { out[i] = val0[i].max(val1[i]); }
                }
                AluOpcode::Slt => {
                    for i in 0..4 { out[i] = if val0[i] < val1[i] { 1.0 } else { 0.0 }; }
                }
                AluOpcode::Sge => {
                    for i in 0..4 { out[i] = if val0[i] >= val1[i] { 1.0 } else { 0.0 }; }
                }
                AluOpcode::Mov => {
                    out = val0;
                }
            }

            match insn.dest {
                ShaderReg::Temp(idx) => {
                    let i = (idx as usize) % 32;
                    for c in 0..4 {
                        if insn.write_mask[c] { r[i][c] = out[c]; }
                    }
                }
                ShaderReg::Export(idx) => {
                    let i = (idx as usize) % 16;
                    for c in 0..4 {
                        if insn.write_mask[c] { exports[i][c] = out[c]; }
                    }
                }
                _ => {}
            }
        }

        // Check if microcode explicitly exported oPos (Export 0)
        if exports[0] != [0.0; 4] {
            clip_pos = exports[0];
        }

        // Perspective divide & Viewport mapping
        let inv_w = if clip_pos[3].abs() > 1e-5 { 1.0 / clip_pos[3] } else { 1.0 };
        let ndc_x = clip_pos[0] * inv_w;
        let ndc_y = clip_pos[1] * inv_w;

        // If coordinates are already in 2D screen space (e.g. XUI coordinates), pass through
        let (screen_x, screen_y) = if clip_pos[0].abs() > 1.0 || clip_pos[1].abs() > 1.0 {
            (clip_pos[0], clip_pos[1])
        } else {
            (ndc_x * vport_xscale + vport_xoffset, ndc_y * vport_yscale + vport_yoffset)
        };

        // Output color: use Export 1 (oColor0) if microcode wrote it, else modulate by constant c4
        let final_color = if exports[1] != [0.0; 4] {
            exports[1]
        } else {
            let c4 = constants[4];
            let has_tint = c4[3] > 0.01;
            if has_tint {
                [
                    v_in.color[0] * c4[0],
                    v_in.color[1] * c4[1],
                    v_in.color[2] * c4[2],
                    v_in.color[3] * c4[3],
                ]
            } else {
                v_in.color
            }
        };

        VertexOutput {
            clip_pos,
            screen_x,
            screen_y,
            color: final_color,
            uv: v_in.uv,
        }
    }

    fn read_reg(
        &self,
        reg: ShaderReg,
        r: &[[f32; 4]; 32],
        constants: &[[f32; 4]; 256],
        v_in: &VertexInput,
    ) -> [f32; 4] {
        match reg {
            ShaderReg::Temp(i) => r[(i as usize) % 32],
            ShaderReg::Constant(i) => constants[(i as usize) % 256],
            ShaderReg::Attribute(0) => v_in.position,
            ShaderReg::Attribute(1) => v_in.color,
            ShaderReg::Attribute(2) => [v_in.uv[0], v_in.uv[1], 0.0, 1.0],
            ShaderReg::Attribute(3) => [v_in.normal[0], v_in.normal[1], v_in.normal[2], 0.0],
            ShaderReg::Attribute(_) => [0.0, 0.0, 0.0, 1.0],
            _ => [0.0; 4],
        }
    }
}

/// ATI Xenos Microcode Pixel Shader Program
#[derive(Debug, Clone)]
pub struct PixelShader {
    pub name: String,
    pub instructions: Vec<AluInstruction>,
    pub raw_bytecode: Vec<u32>,
}

impl PixelShader {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            instructions: Vec::new(),
            raw_bytecode: Vec::new(),
        }
    }

    /// Executes the Pixel Shader on an interpolated fragment
    pub fn execute(
        &self,
        color_in: [f32; 4],
        uv_in: [f32; 2],
        constants: &[[f32; 4]; 256],
        texture: Option<&Texture>,
    ) -> u32 {
        let mut final_r = color_in[0];
        let mut final_g = color_in[1];
        let mut final_b = color_in[2];
        let mut final_a = color_in[3];

        // Sample texture if bound
        if let Some(tex) = texture {
            if tex.width > 0 && tex.height > 0 {
                let u = uv_in[0].clamp(0.0, 1.0);
                let v = uv_in[1].clamp(0.0, 1.0);
                let tx = ((u * (tex.width - 1) as f32) as usize).min(tex.width - 1);
                let ty = ((v * (tex.height - 1) as f32) as usize).min(tex.height - 1);
                let sample_color = tex.pixels[ty * tex.width + tx];

                let ta = ((sample_color >> 24) & 0xFF) as f32 / 255.0;
                let tr = ((sample_color >> 16) & 0xFF) as f32 / 255.0;
                let tg = ((sample_color >> 8) & 0xFF) as f32 / 255.0;
                let tb = (sample_color & 0xFF) as f32 / 255.0;

                final_r *= tr;
                final_g *= tg;
                final_b *= tb;
                final_a *= ta;
            }
        }

        // Multiply by global tint constant c4 if present
        let c4 = constants[4];
        if c4[3] > 0.01 {
            final_r *= c4[0];
            final_g *= c4[1];
            final_b *= c4[2];
            final_a *= c4[3];
        }

        let a = (final_a.clamp(0.0, 1.0) * 255.0) as u32;
        let r = (final_r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (final_g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (final_b.clamp(0.0, 1.0) * 255.0) as u32;

        (a << 24) | (r << 16) | (g << 8) | b
    }
}

/// Factory functions for standard ATI Xenos / Xbox 360 Dashboard shaders
pub fn create_default_xui_vertex_shader() -> VertexShader {
    let mut vs = VertexShader::new("DefaultXuiVertexShader");
    // Matrix transform: oPos.x = dot4(v0, c0), oPos.y = dot4(v0, c1), oPos.z = dot4(v0, c2), oPos.w = dot4(v0, c3)
    vs.instructions.push(AluInstruction {
        opcode: AluOpcode::Dot4,
        dest: ShaderReg::Export(0),
        write_mask: [true, false, false, false],
        src0: ShaderReg::Attribute(0),
        swizzle0: Swizzle::default(),
        src1: ShaderReg::Constant(0),
        swizzle1: Swizzle::default(),
        src2: None,
        swizzle2: Swizzle::default(),
    });
    vs.instructions.push(AluInstruction {
        opcode: AluOpcode::Dot4,
        dest: ShaderReg::Export(0),
        write_mask: [false, true, false, false],
        src0: ShaderReg::Attribute(0),
        swizzle0: Swizzle::default(),
        src1: ShaderReg::Constant(1),
        swizzle1: Swizzle::default(),
        src2: None,
        swizzle2: Swizzle::default(),
    });
    vs.instructions.push(AluInstruction {
        opcode: AluOpcode::Dot4,
        dest: ShaderReg::Export(0),
        write_mask: [false, false, true, false],
        src0: ShaderReg::Attribute(0),
        swizzle0: Swizzle::default(),
        src1: ShaderReg::Constant(2),
        swizzle1: Swizzle::default(),
        src2: None,
        swizzle2: Swizzle::default(),
    });
    vs.instructions.push(AluInstruction {
        opcode: AluOpcode::Dot4,
        dest: ShaderReg::Export(0),
        write_mask: [false, false, false, true],
        src0: ShaderReg::Attribute(0),
        swizzle0: Swizzle::default(),
        src1: ShaderReg::Constant(3),
        swizzle1: Swizzle::default(),
        src2: None,
        swizzle2: Swizzle::default(),
    });
    // Diffuse tint modulation: oColor0 = v1 * c4
    vs.instructions.push(AluInstruction {
        opcode: AluOpcode::Mul,
        dest: ShaderReg::Export(1),
        write_mask: [true, true, true, true],
        src0: ShaderReg::Attribute(1),
        swizzle0: Swizzle::default(),
        src1: ShaderReg::Constant(4),
        swizzle1: Swizzle::default(),
        src2: None,
        swizzle2: Swizzle::default(),
    });
    vs
}

pub fn create_default_xui_pixel_shader() -> PixelShader {
    PixelShader::new("DefaultXuiPixelShader")
}
