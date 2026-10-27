// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2022 Ben Vanik.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/ucode.h                       semantics of every ALU opcode (pseudo-code comments), exports
//   src/xenia/gpu/spirv_shader_translator_alu.cc  Shader Model 3 multiplication (+0 * anything = +0), max/maxa,
//                                                cube operand components, dot product order, mad as mul + add
//   src/xenia/gpu/spirv_shader_translator.cc     loop start/end (count, aL, step), predicated break, call/return
//   src/xenia/gpu/shader_translator.cc           operand and result decoding (see gpu/ucode_disasm.rs)
//
//! CPU interpreter for Xenos shader microcode. It runs one vertex or one pixel at a time: control flow, ALU
//! (vector + scalar), predicates, the address and loop registers, exports and pixel kill. Constants and fetches go
//! through a `ShaderEnv` (the GPU register file and guest memory in the emulator, a mock in tests).

use super::ucode::*;

pub type Vec4 = [f32; 4];

/// Where constants and fetched data come from.
pub trait ShaderEnv {
    /// Float constant `index` (0..511; pixel shaders use 256..511).
    fn float_constant(&self, index: u32) -> Vec4;
    /// Boolean constant `index` (0..255).
    fn bool_constant(&self, index: u32) -> bool;
    /// Loop constant `index` (0..31): count in bits 0..7, initial aL in 8..15, signed step in 16..23.
    fn loop_constant(&self, index: u32) -> u32;
    /// A vertex fetch: `op` is the instruction (format, offset, ...), `full` the full fetch it takes the fetch
    /// constant and stride from (itself unless `op` is a mini fetch), `index` the source register value.
    fn vertex_fetch(&mut self, op: &FetchInstruction, full: &FetchInstruction, index: f32) -> Vec4;
    /// A texture fetch (tfetch*, getters): `coords` as read from the source register (unused components 0).
    fn texture_fetch(&mut self, op: &FetchInstruction, coords: Vec4) -> Vec4;
}

/// Values exported by a shader run; `None` = never written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Exports {
    pub position: Option<Vec4>,
    pub point_size_edge_flag_kill_vertex: Option<Vec4>,
    pub interpolators: [Option<Vec4>; 16],
    pub colors: [Option<Vec4>; 4],
    pub depth: Option<f32>,
    /// Memory export (eA / eM#) writes seen; not executed.
    pub memexport_writes: u32,
}

/// The state of one shader thread.
#[derive(Debug, Clone)]
pub struct ShaderState {
    pub r: [Vec4; 64],
    /// Previous scalar result (ps).
    pub ps: f32,
    pub p0: bool,
    pub a0: i32,
    /// aL of the innermost loop.
    pub al: i32,
    pub killed: bool,
    pub exports: Exports,
    loop_stack: Vec<(u32, i32)>,
    call_stack: Vec<usize>,
}

impl ShaderState {
    /// The default state again, with only the first `temps` registers cleared (a shader that never addresses
    /// registers past them cannot observe the others).
    pub fn reset(&mut self, temps: usize) {
        for r in &mut self.r[..temps.min(64)] {
            *r = [0.0; 4];
        }
        self.ps = 0.0;
        self.p0 = false;
        self.a0 = 0;
        self.al = 0;
        self.killed = false;
        self.exports = Exports::default();
        self.loop_stack.clear();
        self.call_stack.clear();
    }

    /// `reset` for a pixel shader thread: a pixel shader exports only colors, depth and memory (see `alu`), so the
    /// other exports of a state that only ever ran pixel shaders are still unset.
    pub fn reset_pixel(&mut self, temps: usize) {
        for r in &mut self.r[..temps.min(64)] {
            *r = [0.0; 4];
        }
        self.ps = 0.0;
        self.p0 = false;
        self.a0 = 0;
        self.al = 0;
        self.killed = false;
        self.exports.colors = [None; 4];
        self.exports.depth = None;
        self.exports.memexport_writes = 0;
        self.loop_stack.clear();
        self.call_stack.clear();
    }
}

impl Default for ShaderState {
    fn default() -> Self {
        Self { r: [[0.0; 4]; 64], ps: 0.0, p0: false, a0: 0, al: 0, killed: false, exports: Exports::default(), loop_stack: Vec::new(), call_stack: Vec::new() }
    }
}

/// Control flow and ALU/fetch instructions executed before giving up (a shader stuck in a loop).
pub const STEP_LIMIT: u32 = 1_000_000;

/// Shader Model 3 multiplication: +0 or a denormal times anything (even NaN or infinity) is +0.
pub fn mul_sm3(a: f32, b: f32) -> f32 {
    if a.abs() < f32::MIN_POSITIVE || b.abs() < f32::MIN_POSITIVE {
        0.0
    } else {
        a * b
    }
}

fn clamp01(v: f32) -> f32 {
    // NClamp: NaN becomes 0
    if v >= 1.0 { 1.0 } else if v > 0.0 { v } else { 0.0 }
}

fn bool_f(b: bool) -> f32 {
    if b { 1.0 } else { 0.0 }
}

/// a0 = (int)clamp(floor(v), -256, 255) (NaN -> -256, as in the MSDN clamp).
fn address_from(v: f32) -> i32 {
    let f = v.floor();
    if !(f >= -256.0) { -256 } else if f > 255.0 { 255 } else { f as i32 }
}

/// Runs a shader on `state` until it ends. `words` is the whole microcode.
pub fn run(words: &[u32], shader_type: ShaderType, state: &mut ShaderState, env: &mut dyn ShaderEnv) -> Result<(), String> {
    CompiledShader::compile(words, shader_type, env).run(state, env)
}

/// A source operand decoded once.
#[derive(Debug, Clone, Copy)]
enum Operand {
    /// A constant at a fixed index: its final value (swizzle, abs and negate applied), read when compiling.
    Constant(Vec4),
    /// A temporary register (relative to aL when `relative`) or a constant indexed by a0 / aL, read at run time.
    Register { temp: bool, index: u32, relative: bool, a0: bool, base: u32, abs: bool, swizzle: [u8; 4], negate: bool },
}

impl Operand {
    /// read_source for ALU source `i` (1..=3).
    fn decode(op: &AluInstruction, i: u32, shader_type: ShaderType, env: &dyn ShaderEnv) -> Self {
        let reg = op.src_reg(i);
        let swizzle_bits = op.src_swizzle(i);
        let swizzle: [u8; 4] = std::array::from_fn(|c| swizzled_component_index(swizzle_bits, c as u32) as u8);
        let negate = op.src_negate(i);
        let base = if shader_type == ShaderType::Pixel { 256 } else { 0 };
        if op.src_is_temp(i) {
            return Operand::Register { temp: true, index: reg & 0x3F, relative: reg & 0x40 != 0, a0: false, base, abs: reg & 0x80 != 0, swizzle, negate };
        }
        let abs = op.abs_constants();
        if op.src_const_is_addressed(i) {
            return Operand::Register { temp: false, index: reg, relative: true, a0: op.is_const_address_register_relative(), base, abs, swizzle, negate };
        }
        let value = env.float_constant(base + (reg & 0xFF));
        Operand::Constant(std::array::from_fn(|c| {
            let v = value[swizzle[c] as usize];
            let v = if abs { v.abs() } else { v };
            if negate { -v } else { v }
        }))
    }

    #[inline(always)]
    fn read(&self, state: &ShaderState, env: &dyn ShaderEnv) -> Vec4 {
        match *self {
            Operand::Constant(v) => v,
            Operand::Register { temp, index, relative, a0, base, abs, swizzle, negate } => {
                let value = if temp {
                    state.r[temp_index(index, relative, state)]
                } else {
                    let index = index as i32 + if a0 { state.a0 } else { state.al };
                    env.float_constant(base + (index & 0xFF) as u32)
                };
                let mut out = [value[swizzle[0] as usize], value[swizzle[1] as usize], value[swizzle[2] as usize], value[swizzle[3] as usize]];
                if abs {
                    out = out.map(f32::abs);
                }
                if negate {
                    out = out.map(|v| -v);
                }
                out
            }
        }
    }
}

/// Where the scalar operation's operands come from (scalar_operands).
#[derive(Debug, Clone, Copy)]
enum ScalarSource {
    None,
    /// `a` = W and `b` = component `second` of source 3.
    Source3 { second: usize },
    /// The constant-register forms: `a` from a constant (fixed value, or indexed by a0 / aL) and `b` from a temporary.
    ConstTemp { constant: ConstScalar, temp: usize, temp_component: usize, abs: bool, negate: bool },
}

#[derive(Debug, Clone, Copy)]
enum ConstScalar {
    Fixed(f32),
    Indexed { index: u32, a0: bool, base: u32, component: usize },
}

/// Where an ALU instruction writes, decoded once.
#[derive(Debug, Clone, Copy)]
enum AluDest {
    /// Temporaries: the vector result to (index, relative) under its mask, the scalar result likewise.
    Temps { vector: (u32, bool), vector_mask: u32, scalar: (u32, bool), scalar_mask: u32 },
    /// An export to register `reg`: vector / scalar result components, constant 1 components, all written ones.
    Export { reg: u32, vector_mask: u32, scalar_mask: u32, one_mask: u32, mask: u32 },
}

/// An ALU instruction decoded once.
#[derive(Debug, Clone, Copy)]
struct CompiledAlu {
    predicate: Option<bool>,
    vector_clamp: bool,
    scalar_clamp: bool,
    dest: AluDest,
    vector_opcode: u32,
    vector_operands: usize,
    scalar_opcode: u32,
    sources: [Operand; 3],
    scalar: ScalarSource,
}

impl CompiledAlu {
    fn decode(op: AluInstruction, shader_type: ShaderType, env: &dyn ShaderEnv) -> Self {
        let vector_opcode = op.vector_opcode();
        let scalar_opcode = op.scalar_opcode();
        let sources = [1, 2, 3].map(|i| Operand::decode(&op, i, shader_type, env));
        let info = &SCALAR_OPCODES[scalar_opcode as usize];
        let scalar = if info.operand_count == 0 {
            ScalarSource::None
        } else if info.operand_count == 2 {
            let swizzle = op.src_swizzle(3);
            let (abs, negate) = (op.abs_constants(), op.src_negate(3));
            let base = if shader_type == ShaderType::Pixel { 256 } else { 0 };
            let component = swizzled_component_index(swizzle, 3) as usize;
            let constant = if op.src_const_is_addressed(3) {
                ConstScalar::Indexed { index: op.src_reg(3), a0: op.is_const_address_register_relative(), base, component }
            } else {
                let v = env.float_constant(base + (op.src_reg(3) & 0xFF))[component];
                let v = if abs { v.abs() } else { v };
                ConstScalar::Fixed(if negate { -v } else { v })
            };
            ScalarSource::ConstTemp {
                constant,
                temp: (op.scalar_const_reg_op_src_temp_reg() & 63) as usize,
                temp_component: swizzled_component_index(swizzle, 0) as usize,
                abs,
                negate,
            }
        } else {
            ScalarSource::Source3 { second: if VECTOR_OPCODES[vector_opcode as usize].operand_count() == 3 { 2 } else { 0 } }
        };
        let dest = if op.is_export() {
            let (vector_mask, scalar_mask) = (op.vector_op_result_write_mask(), op.scalar_op_result_write_mask());
            let (zero_mask, one_mask) = (op.constant_0_write_mask(), op.constant_1_write_mask());
            AluDest::Export { reg: op.vector_dest(), vector_mask, scalar_mask, one_mask, mask: vector_mask | scalar_mask | zero_mask | one_mask }
        } else {
            AluDest::Temps {
                vector: (op.vector_dest(), op.is_vector_dest_relative()),
                vector_mask: op.vector_write_mask(),
                scalar: (op.scalar_dest(), op.is_scalar_dest_relative()),
                scalar_mask: op.scalar_write_mask(),
            }
        };
        Self {
            predicate: op.is_predicated().then(|| op.predicate_condition()),
            vector_clamp: op.vector_clamp(),
            scalar_clamp: op.scalar_clamp(),
            dest,
            vector_opcode,
            vector_operands: VECTOR_OPCODES[vector_opcode as usize].operand_count() as usize,
            scalar_opcode,
            sources,
            scalar,
        }
    }

    #[inline(always)]
    fn scalar_operands(&self, state: &ShaderState, env: &dyn ShaderEnv, source3: &Vec4) -> (f32, f32) {
        match self.scalar {
            ScalarSource::None => (0.0, 0.0),
            ScalarSource::Source3 { second } => {
                // source 3 was just read when the vector operation takes three operands (nothing written since)
                let v = if self.vector_operands == 3 { *source3 } else { self.sources[2].read(state, env) };
                (v[3], v[second])
            }
            ScalarSource::ConstTemp { constant, temp, temp_component, abs, negate } => {
                let fix = |v: f32| {
                    let v = if abs { v.abs() } else { v };
                    if negate { -v } else { v }
                };
                let a = match constant {
                    ConstScalar::Fixed(v) => v,
                    ConstScalar::Indexed { index, a0, base, component } => {
                        let index = index as i32 + if a0 { state.a0 } else { state.al };
                        fix(env.float_constant(base + (index & 0xFF) as u32)[component])
                    }
                };
                (a, fix(state.r[temp][temp_component]))
            }
        }
    }
}

/// A shader decoded once for a draw: control flow, and the ALU instructions with the constants they read at fixed
/// indices already resolved. Running it is the same as `run` on the same microcode and constants.
pub struct CompiledShader {
    shader_type: ShaderType,
    words: Vec<u32>,
    cf: Vec<CfInstruction>,
    alu: Vec<Option<CompiledAlu>>,
    /// Temporary registers the shader can address: one past the highest index it names, all 64 when it addresses
    /// any relatively.
    temps: usize,
}

impl CompiledShader {
    pub fn compile(words: &[u32], shader_type: ShaderType, env: &dyn ShaderEnv) -> Self {
        let cf: Vec<CfInstruction> = (0..control_flow_length(words) * 2)
            .map(|index| {
                let pair = index / 2;
                unpack_cf_pair([words[pair * 3], words[pair * 3 + 1], words[pair * 3 + 2]])[index % 2]
            })
            .collect();
        let mut alu = vec![None; words.len() / 3];
        let mut temps = 0usize;
        let mut name = |index: u32, relative: bool| temps = if relative { 64 } else { temps.max((index & 63) as usize + 1) };
        for c in cf.iter().filter(|c| c.opcode().is_exec()) {
            let mut sequence = c.exec_sequence();
            for offset in c.exec_address()..c.exec_address() + c.exec_count() {
                let base = offset as usize * 3;
                if base + 3 <= words.len() {
                    let w = [words[base], words[base + 1], words[base + 2]];
                    if sequence & 1 == 0 {
                        let op = AluInstruction(w);
                        for i in 1..=3 {
                            if op.src_is_temp(i) {
                                name(op.src_reg(i) & 0x3F, op.src_reg(i) & 0x40 != 0);
                            }
                        }
                        name(op.scalar_const_reg_op_src_temp_reg(), false);
                        if !op.is_export() {
                            name(op.vector_dest(), op.is_vector_dest_relative());
                        }
                        name(op.scalar_dest(), op.is_scalar_dest_relative());
                        alu[offset as usize] = Some(CompiledAlu::decode(op, shader_type, env));
                    } else {
                        let op = FetchInstruction(w);
                        name(op.src(), op.is_src_relative());
                        name(op.dest(), op.is_dest_relative());
                    }
                }
                sequence >>= 2;
            }
        }
        Self { shader_type, words: words.to_vec(), cf, alu, temps }
    }

    /// See `ShaderState::reset`.
    pub fn temps(&self) -> usize {
        self.temps
    }

    /// Runs the shader on `state` until it ends.
    pub fn run(&self, state: &mut ShaderState, env: &mut dyn ShaderEnv) -> Result<(), String> {
        let words = &self.words[..];
        let shader_type = self.shader_type;
        let cf_count = self.cf.len();
        let mut previous_vfetch_full = FetchInstruction::default();
        let mut steps = 0u32;
        let mut pc = 0usize;
        while pc < cf_count {
            steps += 1;
            if steps > STEP_LIMIT {
                return Err(format!("shader did not end after {} steps", STEP_LIMIT));
            }
            let cf = self.cf[pc];
            let mut next = pc + 1;
            let op = cf.opcode();
            match op {
                CfOpcode::Nop | CfOpcode::Alloc | CfOpcode::MarkVsFetchDone => {}
                _ if op.is_exec() => {
                    let run_body = match op {
                        CfOpcode::Exec | CfOpcode::ExecEnd => true,
                        CfOpcode::CondExecPred | CfOpcode::CondExecPredEnd => state.p0 == cf.condition(),
                        _ => env.bool_constant(cf.bool_address()) == cf.condition(),
                    };
                    if run_body {
                        let mut sequence = cf.exec_sequence();
                        for offset in cf.exec_address()..cf.exec_address() + cf.exec_count() {
                            let base = offset as usize * 3;
                            let w = words.get(base..base + 3).ok_or_else(|| format!("exec instruction {} past the end", offset))?;
                            if sequence & 1 != 0 {
                                fetch(&FetchInstruction([w[0], w[1], w[2]]), state, env, &mut previous_vfetch_full);
                            } else if let Some(alu_op) = &self.alu[offset as usize] {
                                alu(alu_op, shader_type, state, env);
                            }
                            sequence >>= 2;
                            steps += 1;
                        }
                    }
                    // the end of the shader does not depend on the condition
                    if op.ends_shader() {
                        return Ok(());
                    }
                }
                CfOpcode::LoopStart => {
                    let constant = env.loop_constant(cf.loop_id());
                    let count = constant & 0xFF;
                    if !cf.loop_is_repeat() {
                        // push: the outer aL comes back on endloop
                        state.loop_stack.push((count, state.al));
                        state.al = ((constant >> 8) & 0xFF) as i32;
                    } else {
                        state.loop_stack.push((count, state.al));
                    }
                    if count == 0 {
                        // skip the loop (the counter is checked at its end) to the instruction after endloop; Xenia
                        // leaves this loop's entry on its stack, here it is popped as endloop would
                        let (_, al) = state.loop_stack.pop().unwrap();
                        state.al = al;
                        next = cf.target_address() as usize;
                    }
                }
                CfOpcode::LoopEnd => {
                    let constant = env.loop_constant(cf.loop_id());
                    let (count, outer_al) = state.loop_stack.last().copied().unwrap_or((1, state.al));
                    let count = count.saturating_sub(1);
                    let break_now = count == 0 || (cf.loop_is_predicated_break() && state.p0 == cf.condition());
                    if break_now {
                        state.loop_stack.pop();
                        state.al = outer_al;
                    } else {
                        if let Some(top) = state.loop_stack.last_mut() {
                            top.0 = count;
                        }
                        state.al += ((constant >> 16) & 0xFF) as u8 as i8 as i32;
                        next = cf.target_address() as usize;
                    }
                }
                CfOpcode::CondCall | CfOpcode::CondJmp => {
                    let taken = if cf.is_unconditional() {
                        true
                    } else if cf.is_predicated() {
                        state.p0 == cf.condition()
                    } else {
                        env.bool_constant(cf.bool_address()) == cf.condition()
                    };
                    if taken {
                        if op == CfOpcode::CondCall {
                            state.call_stack.push(pc + 1);
                        }
                        next = cf.target_address() as usize;
                    }
                }
                CfOpcode::Return => match state.call_stack.pop() {
                    Some(r) => next = r,
                    None => return Ok(()),
                },
                _ => return Err(format!("unhandled control flow opcode {:?}", op)),
            }
            pc = next;
        }
        Ok(())
    }
}

/// The number of control flow pairs (Shader::AnalyzeUcode): up to the first exec's address.
pub fn control_flow_length(words: &[u32]) -> usize {
    let mut bound = words.len() / 3;
    let mut i = 0;
    while i < bound {
        for cf in unpack_cf_pair([words[i * 3], words[i * 3 + 1], words[i * 3 + 2]]) {
            if cf.opcode().is_exec() {
                bound = bound.min(cf.exec_address() as usize);
            }
        }
        i += 1;
    }
    bound
}

fn temp_index(index: u32, relative: bool, state: &ShaderState) -> usize {
    if relative { ((index as i32 + state.al) & 63) as usize } else { (index & 63) as usize }
}

/// One vector ALU operation. Returns the result and updates p0 / a0 / kill as the opcode does.
fn vector_op(opcode: u32, s: &[Vec4; 3], state: &mut ShaderState) -> Vec4 {
    let [a, b, c] = s;
    #[inline(always)]
    fn per_component(a: &Vec4, b: &Vec4, c: &Vec4, f: impl Fn(f32, f32, f32) -> f32) -> Vec4 {
        [f(a[0], b[0], c[0]), f(a[1], b[1], c[1]), f(a[2], b[2], c[2]), f(a[3], b[3], c[3])]
    }

    let splat = |v: f32| [v; 4];
    match opcode {
        0 => per_component(a, b, c, |x, y, _| x + y),
        1 => per_component(a, b, c, |x, y, _| mul_sm3(x, y)),
        2 => per_component(a, b, c, |x, y, _| if x >= y { x } else { y }),
        3 => per_component(a, b, c, |x, y, _| if x < y { x } else { y }),
        4 => per_component(a, b, c, |x, y, _| bool_f(x == y)),
        5 => per_component(a, b, c, |x, y, _| bool_f(x > y)),
        6 => per_component(a, b, c, |x, y, _| bool_f(x >= y)),
        7 => per_component(a, b, c, |x, y, _| bool_f(x != y)),
        8 => per_component(a, b, c, |x, _, _| x - x.floor()),
        9 => per_component(a, b, c, |x, _, _| x.trunc()),
        10 => per_component(a, b, c, |x, _, _| x.floor()),
        11 => per_component(a, b, c, |x, y, z| mul_sm3(x, y) + z),
        12 => per_component(a, b, c, |x, y, z| if x == 0.0 { y } else { z }),
        13 => per_component(a, b, c, |x, y, z| if x >= 0.0 { y } else { z }),
        14 => per_component(a, b, c, |x, y, z| if x > 0.0 { y } else { z }),
        15 => splat(((mul_sm3(a[0], b[0]) + mul_sm3(a[1], b[1])) + mul_sm3(a[2], b[2])) + mul_sm3(a[3], b[3])),
        16 => splat((mul_sm3(a[0], b[0]) + mul_sm3(a[1], b[1])) + mul_sm3(a[2], b[2])),
        17 => splat((mul_sm3(a[0], b[0]) + mul_sm3(a[1], b[1])) + c[0]),
        18 => {
            // operand 0 is .z_xy: z, x, y of the cube vector are its components 0, 2 and 3
            let (z, x, y) = (a[0], a[2], a[3]);
            if z.abs() >= x.abs() && z.abs() >= y.abs() {
                [-y, if z < 0.0 { -x } else { x }, 2.0 * z, if z < 0.0 { 5.0 } else { 4.0 }]
            } else if y.abs() >= x.abs() {
                [if y < 0.0 { -z } else { z }, x, 2.0 * y, if y < 0.0 { 3.0 } else { 2.0 }]
            } else {
                [-y, if x < 0.0 { z } else { -z }, 2.0 * x, if x < 0.0 { 1.0 } else { 0.0 }]
            }
        }
        19 => {
            let v = if a[0] > a[1] && a[0] > a[2] && a[0] > a[3] {
                a[0]
            } else if a[1] > a[2] && a[1] > a[3] {
                a[1]
            } else if a[2] > a[3] {
                a[2]
            } else {
                a[3]
            };
            splat(v)
        }
        20..=23 => {
            let test = |x: f32, y: f32| match opcode {
                20 => x == 0.0 && y == 0.0,
                21 => x == 0.0 && y != 0.0,
                22 => x == 0.0 && y > 0.0,
                _ => x == 0.0 && y >= 0.0,
            };
            state.p0 = test(a[3], b[3]);
            splat(if test(a[0], b[0]) { 0.0 } else { a[0] + 1.0 })
        }
        24..=27 => {
            let test = |x: f32, y: f32| match opcode {
                24 => x == y,
                25 => x > y,
                26 => x >= y,
                _ => x != y,
            };
            let kill = (0..4).any(|i| test(a[i], b[i]));
            state.killed |= kill;
            splat(bool_f(kill))
        }
        28 => [1.0, mul_sm3(a[1], b[1]), a[2], b[3]],
        29 => {
            state.a0 = address_from(a[3] + 0.5);
            per_component(a, b, c, |x, y, _| if x >= y { x } else { y })
        }
        // opcode_30 / opcode_31: no operation defined
        _ => splat(0.0),
    }
}

/// One scalar ALU operation; updates p0 / a0 / kill as the opcode does.
fn scalar_op(opcode: u32, a: f32, b: f32, state: &mut ShaderState) -> f32 {
    let ps = state.ps;
    let setp = |state: &mut ShaderState, cond: bool, else_value: f32| -> f32 {
        state.p0 = cond;
        if cond { 0.0 } else { else_value }
    };
    let kill = |state: &mut ShaderState, cond: bool| -> f32 {
        state.killed |= cond;
        bool_f(cond)
    };
    let clamp_inf = |t: f32, neg: f32, pos: f32| if t == f32::NEG_INFINITY { neg } else if t == f32::INFINITY { pos } else { t };
    let rcp = |x: f32| if x == 1.0 { 1.0 } else { 1.0 / x };
    let rsq = |x: f32| if x == 1.0 { 1.0 } else { 1.0 / x.sqrt() };
    match opcode {
        0 => a + b,
        1 => a + ps,
        2 => mul_sm3(a, b),
        3 => mul_sm3(a, ps),
        4 => {
            if ps == -f32::MAX || !ps.is_finite() || !b.is_finite() || b <= 0.0 { -f32::MAX } else { mul_sm3(a, ps) }
        }
        5 => if a >= b { a } else { b },
        6 => if a < b { a } else { b },
        7 => bool_f(a == 0.0),
        8 => bool_f(a > 0.0),
        9 => bool_f(a >= 0.0),
        10 => bool_f(a != 0.0),
        11 => a - a.floor(),
        12 => a.trunc(),
        13 => a.floor(),
        14 => if a == 0.0 { 1.0 } else { a.exp2() },
        15 => {
            let t = if a == 1.0 { 0.0 } else { a.log2() };
            if t == f32::NEG_INFINITY { -f32::MAX } else { t }
        }
        16 => if a == 1.0 { 0.0 } else { a.log2() },
        17 => clamp_inf(rcp(a), -f32::MAX, f32::MAX),
        18 => clamp_inf(rcp(a), -0.0, 0.0),
        19 => rcp(a),
        20 => clamp_inf(rsq(a), -f32::MAX, f32::MAX),
        21 => clamp_inf(rsq(a), -0.0, 0.0),
        22 => rsq(a),
        23 | 24 => {
            state.a0 = address_from(if opcode == 23 { a + 0.5 } else { a });
            if a >= b { a } else { b }
        }
        25 => a - b,
        26 => a - ps,
        27 => setp(state, a == 0.0, 1.0),
        28 => setp(state, a != 0.0, 1.0),
        29 => setp(state, a > 0.0, 1.0),
        30 => setp(state, a >= 0.0, 1.0),
        31 => {
            if a == 1.0 {
                setp(state, true, 0.0)
            } else {
                setp(state, false, if a == 0.0 { 1.0 } else { a })
            }
        }
        32 => setp(state, a - 1.0 <= 0.0, a - 1.0),
        33 => {
            state.p0 = false;
            f32::MAX
        }
        34 => setp(state, a == 0.0, a),
        35 => kill(state, a == 0.0),
        36 => kill(state, a > 0.0),
        37 => kill(state, a >= 0.0),
        38 => kill(state, a != 0.0),
        39 => kill(state, a == 1.0),
        40 => a.sqrt(),
        42 | 43 => mul_sm3(a, b),
        44 | 45 => a + b,
        46 | 47 => a - b,
        48 => a.sin(),
        49 => a.cos(),
        50 => ps,
        // opcode_41, opcode_51..63: no operation defined
        _ => 0.0,
    }
}

/// Writes `value` to temporary register `index` under `mask`.
fn write_temp(state: &mut ShaderState, index: usize, mask: u32, value: Vec4) {
    match mask {
        0 => return,
        0xF => {
            state.r[index] = value;
            return;
        }
        _ => {}
    }
    for (c, v) in value.iter().enumerate() {
        if mask & (1 << c) != 0 {
            state.r[index][c] = *v;
        }
    }
}

/// Merges `value` into an export under `mask` (components never written stay 0).
fn write_export(slot: &mut Option<Vec4>, mask: u32, value: Vec4) {
    if mask == 0 {
        return;
    }
    let out = slot.get_or_insert([0.0; 4]);
    for (c, v) in value.iter().enumerate() {
        if mask & (1 << c) != 0 {
            out[c] = *v;
        }
    }
}

/// One ALU instruction: both operations read their sources first, then write.
fn alu(compiled: &CompiledAlu, shader_type: ShaderType, state: &mut ShaderState, env: &dyn ShaderEnv) {
    if compiled.predicate.is_some_and(|condition| state.p0 != condition) {
        return;
    }
    let (vector_opcode, scalar_opcode) = (compiled.vector_opcode, compiled.scalar_opcode);
    let mut sources = [[0.0f32; 4]; 3];
    for (i, source) in sources.iter_mut().enumerate().take(compiled.vector_operands) {
        *source = compiled.sources[i].read(state, env);
    }
    let (a, b) = compiled.scalar_operands(state, env, &sources[2]);

    let mut vector = vector_op(vector_opcode, &sources, state);
    let mut scalar = scalar_op(scalar_opcode, a, b, state);
    state.ps = scalar;
    if compiled.vector_clamp {
        vector = vector.map(clamp01);
    }
    if compiled.scalar_clamp {
        scalar = clamp01(scalar);
    }

    let (reg, vmask, smask, one_mask, mask) = match compiled.dest {
        AluDest::Temps { vector: (vd, vrel), vector_mask, scalar: (sd, srel), scalar_mask } => {
            write_temp(state, temp_index(vd, vrel, state), vector_mask, vector);
            write_temp(state, temp_index(sd, srel, state), scalar_mask, [scalar; 4]);
            return;
        }
        AluDest::Export { reg, vector_mask, scalar_mask, one_mask, mask } => (reg, vector_mask, scalar_mask, one_mask, mask),
    };
    // an export: vector and scalar components, constant 0 and 1 components, into the register in vector_dest
    let mut value = [0.0; 4];
    for c in 0..4 {
        value[c] = if vmask & (1 << c) != 0 {
            vector[c]
        } else if smask & (1 << c) != 0 {
            scalar
        } else if one_mask & (1 << c) != 0 {
            1.0
        } else {
            0.0
        };
    }
    let ex = &mut state.exports;
    match (shader_type, reg) {
        (_, export::EXPORT_ADDRESS) | (_, export::EXPORT_DATA_0..=export::EXPORT_DATA_4) => ex.memexport_writes += 1,
        (ShaderType::Vertex, export::VS_INTERPOLATOR_0..=export::VS_INTERPOLATOR_15) => write_export(&mut ex.interpolators[reg as usize], mask, value),
        (ShaderType::Vertex, export::VS_POSITION) => write_export(&mut ex.position, mask, value),
        (ShaderType::Vertex, export::VS_POINT_SIZE_EDGE_FLAG_KILL_VERTEX) => write_export(&mut ex.point_size_edge_flag_kill_vertex, mask, value),
        (ShaderType::Pixel, export::PS_COLOR_0..=export::PS_COLOR_3) => write_export(&mut ex.colors[reg as usize], mask, value),
        (ShaderType::Pixel, export::PS_DEPTH) => {
            if mask & 1 != 0 {
                ex.depth = Some(value[0]);
            }
        }
        _ => {}
    }
}

/// One fetch instruction: the result goes to a temporary register through the destination swizzle.
fn fetch(op: &FetchInstruction, state: &mut ShaderState, env: &mut dyn ShaderEnv, previous_vfetch_full: &mut FetchInstruction) {
    if op.is_predicated() && state.p0 != op.predicate_condition() {
        return;
    }
    let value = match op.opcode() {
        FetchOpcode::VertexFetch => {
            let full = if op.vf_is_mini_fetch() { *previous_vfetch_full } else { *op };
            if !op.vf_is_mini_fetch() {
                *previous_vfetch_full = *op;
            }
            let src = state.r[temp_index(full.src(), full.is_src_relative(), state)];
            env.vertex_fetch(op, &full, src[(full.vf_src_swizzle() & 3) as usize])
        }
        FetchOpcode::SetTextureLod | FetchOpcode::SetTextureGradientsHorz | FetchOpcode::SetTextureGradientsVert => {
            // texture LOD / gradient state for later fetches: not modeled
            return;
        }
        FetchOpcode::Unknown(_) => return,
        _ => {
            let src = state.r[temp_index(op.src(), op.is_src_relative(), state)];
            let count = fetch_dimension_component_count(op.tf_dimension()) as usize;
            let swizzle = op.tf_src_swizzle();
            let mut coords = [0.0; 4];
            for (j, c) in coords.iter_mut().enumerate().take(count.max(2)) {
                *c = src[((swizzle >> (2 * j)) & 3) as usize];
            }
            env.texture_fetch(op, coords)
        }
    };
    let dest = temp_index(op.dest(), op.is_dest_relative(), state);
    let swizzle = op.dest_swizzle();
    for c in 0..4 {
        match fetch_dest_component_swizzle(swizzle, c as u32) {
            s @ 0..=3 => state.r[dest][c] = value[s as usize],
            5 => state.r[dest][c] = 1.0,
            7 => {}
            _ => state.r[dest][c] = 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::ucode_test::{alu as enc, cf, exec, pack, shader, Alu};

    struct Mock {
        c: Vec<Vec4>,
        bools: [bool; 256],
        loops: [u32; 32],
        vertex_fetches: Vec<(u32, f32)>,
        texture_coords: Vec<Vec4>,
    }

    impl Mock {
        fn new() -> Self {
            Self { c: vec![[0.0; 4]; 512], bools: [false; 256], loops: [0; 32], vertex_fetches: Vec::new(), texture_coords: Vec::new() }
        }
    }

    impl ShaderEnv for Mock {
        fn float_constant(&self, index: u32) -> Vec4 {
            self.c[index as usize]
        }
        fn bool_constant(&self, index: u32) -> bool {
            self.bools[index as usize]
        }
        fn loop_constant(&self, index: u32) -> u32 {
            self.loops[index as usize]
        }
        fn vertex_fetch(&mut self, op: &FetchInstruction, full: &FetchInstruction, index: f32) -> Vec4 {
            self.vertex_fetches.push((full.vf_fetch_constant_index(), index));
            [index * 10.0, op.vf_offset() as f32, 2.0, 3.0]
        }
        fn texture_fetch(&mut self, _op: &FetchInstruction, coords: Vec4) -> Vec4 {
            self.texture_coords.push(coords);
            [0.25, 0.5, 0.75, 1.0]
        }
    }

    const EXEC: u32 = 1;
    const EXEC_END: u32 = 2;
    const CLEAN: u32 = 1 << 9;

    /// exece over `body` (ALU unless the sequence says fetch), starting at instruction 1.
    fn run_body(ty: ShaderType, body: &[[u32; 3]], sequence: u32, env: &mut Mock, setup: impl FnOnce(&mut ShaderState)) -> ShaderState {
        let mut parts = vec![pack(exec(EXEC_END, 1, body.len() as u32, sequence, CLEAN), cf(0, 0, 0))];
        parts.extend_from_slice(body);
        let mut state = ShaderState::default();
        setup(&mut state);
        run(&shader(&parts), ty, &mut state, env).unwrap();
        state
    }

    /// r1 = vector opcode (r2, r3, r4).
    fn vector(opcode: u32, a: Vec4, b: Vec4, c: Vec4) -> (Vec4, ShaderState) {
        let i = enc(Alu { vdst: 1, vmask: 0xF, vop: opcode, sop: SCALAR_RETAIN_PREV, reg: [2, 3, 4], temp: [true; 3], ..Default::default() });
        let s = run_body(ShaderType::Vertex, &[i], 0, &mut Mock::new(), |s| {
            s.r[2] = a;
            s.r[3] = b;
            s.r[4] = c;
        });
        (s.r[1], s)
    }

    /// r5 = scalar opcode (a = r4.w, b = r4.x), with ps preset.
    fn scalar(opcode: u32, a: f32, b: f32, ps: f32) -> (f32, ShaderState) {
        let i = enc(Alu { vop: VECTOR_MAX, sop: opcode, sdst: 5, smask: 0xF, reg: [0, 0, 4], temp: [true; 3], ..Default::default() });
        let s = run_body(ShaderType::Vertex, &[i], 0, &mut Mock::new(), |s| {
            s.r[4] = [b, 0.0, 0.0, a];
            s.ps = ps;
        });
        assert_eq!(s.r[5], [s.r[5][0]; 4], "scalar results are replicated");
        (s.r[5][0], s)
    }

    #[test]
    fn vector_opcodes_follow_the_ucode_pseudocode() {
        let a = [1.5, -2.0, 0.0, 4.0];
        let b = [2.0, -2.0, 3.0, -1.0];
        let c = [0.5, 1.0, -1.0, 2.0];
        let cases: [(u32, Vec4); 22] = [
            (0, [3.5, -4.0, 3.0, 3.0]),   // add
            (1, [3.0, 4.0, 0.0, -4.0]),   // mul
            (2, [2.0, -2.0, 3.0, 4.0]),   // max
            (3, [1.5, -2.0, 0.0, -1.0]),  // min
            (4, [0.0, 1.0, 0.0, 0.0]),    // seq
            (5, [0.0, 0.0, 0.0, 1.0]),    // sgt
            (6, [0.0, 1.0, 0.0, 1.0]),    // sge
            (7, [1.0, 0.0, 1.0, 1.0]),    // sne
            (8, [0.5, 0.0, 0.0, 0.0]),    // frc
            (9, [1.0, -2.0, 0.0, 4.0]),   // trunc
            (10, [1.0, -2.0, 0.0, 4.0]),  // floor
            (11, [3.5, 5.0, -1.0, -2.0]), // mad
            (12, [0.5, 1.0, 3.0, 2.0]),   // cndeq
            (13, [2.0, 1.0, 3.0, -1.0]),  // cndge
            (14, [2.0, 1.0, -1.0, -1.0]), // cndgt
            (15, [3.0; 4]),               // dp4
            (16, [7.0; 4]),               // dp3
            (17, [7.5; 4]),               // dp2add = a.x*b.x + a.y*b.y + c.x
            (19, [4.0; 4]),               // max4
            (28, [1.0, 4.0, 0.0, -1.0]),  // dst
            (29, [2.0, -2.0, 3.0, 4.0]),  // maxa
            (30, [0.0; 4]),               // opcode_30: undefined
        ];
        for (opcode, expected) in cases {
            assert_eq!(vector(opcode, a, b, c).0, expected, "{}", VECTOR_OPCODES[opcode as usize].name);
        }
        assert_eq!(vector(9, [-1.5, 2.5, -0.5, 0.0], b, c).0, [-1.0, 2.0, 0.0, 0.0], "trunc rounds toward zero");
    }

    #[test]
    fn multiplication_follows_shader_model_3() {
        let (r, _) = vector(1, [0.0, -0.0, f32::INFINITY, 1e-40], [f32::INFINITY, f32::NAN, 0.0, 5.0], [0.0; 4]);
        assert!(r.iter().all(|v| *v == 0.0 && v.is_sign_positive()), "{:?}", r);
        assert_eq!(vector(11, [0.0; 4], [f32::INFINITY; 4], [2.0; 4]).0, [2.0; 4], "mad");
        assert_eq!(vector(15, [0.0, 1.0, 1.0, 1.0], [f32::NAN, 1.0, 1.0, 1.0], [0.0; 4]).0, [3.0; 4], "dp4");
        assert_eq!(scalar(2, 0.0, f32::INFINITY, 0.0).0, 0.0, "muls");
    }

    #[test]
    fn cube_maps_the_major_axis() {
        // operand 0 is .z_xy of the direction (1, 2, -5): Z major, negative
        assert_eq!(vector(18, [-5.0, 0.0, 1.0, 2.0], [0.0; 4], [0.0; 4]).0, [-2.0, -1.0, -10.0, 5.0]);
        // (3, 1, 2): X major, positive
        assert_eq!(vector(18, [2.0, 0.0, 3.0, 1.0], [0.0; 4], [0.0; 4]).0, [-1.0, -2.0, 6.0, 0.0]);
    }

    #[test]
    fn vector_predicates_kills_and_address_register() {
        let (r, s) = vector(20, [0.0; 4], [0.0; 4], [0.0; 4]);
        assert_eq!((r, s.p0), ([0.0; 4], true), "setp_eq_push");
        let (r, s) = vector(20, [3.0, 0.0, 0.0, 1.0], [0.0; 4], [0.0; 4]);
        assert_eq!((r, s.p0), ([4.0; 4], false));
        let (r, s) = vector(26, [0.0; 4], [1.0, 1.0, 1.0, 0.0], [0.0; 4]);
        assert_eq!((r, s.killed), ([1.0; 4], true), "kill_ge");
        let (_, s) = vector(25, [0.0; 4], [1.0; 4], [0.0; 4]);
        assert!(!s.killed, "kill_gt not taken");
        let (_, s) = vector(29, [0.0, 0.0, 0.0, 2.6], [0.0; 4], [0.0; 4]);
        assert_eq!(s.a0, 3, "maxa: floor(w + 0.5)");
        let (_, s) = vector(29, [0.0, 0.0, 0.0, 1000.0], [0.0; 4], [0.0; 4]);
        assert_eq!(s.a0, 255);
    }

    #[test]
    fn scalar_opcodes_follow_the_ucode_pseudocode() {
        let max = f32::MAX;
        let cases: &[(u32, f32, f32, f32, f32)] = &[
            // (opcode, a, b, ps, expected)
            (0, 1.5, 2.0, 0.0, 3.5),
            (1, 1.0, 9.0, 2.0, 3.0),
            (2, 2.0, 3.0, 0.0, 6.0),
            (3, 2.0, 9.0, 4.0, 8.0),
            (4, 3.0, 1.0, 2.0, 6.0),
            (4, 3.0, 0.0, 2.0, -max),
            (5, 1.0, 2.0, 0.0, 2.0),
            (6, 1.0, 2.0, 0.0, 1.0),
            (7, 0.0, 0.0, 0.0, 1.0),
            (8, 0.0, 0.0, 0.0, 0.0),
            (9, 0.0, 0.0, 0.0, 1.0),
            (10, 2.0, 0.0, 0.0, 1.0),
            (11, -1.25, 0.0, 0.0, 0.75),
            (12, -1.5, 0.0, 0.0, -1.0),
            (13, -1.5, 0.0, 0.0, -2.0),
            (14, 3.0, 0.0, 0.0, 8.0),
            (14, 0.0, 0.0, 0.0, 1.0),
            (15, 0.0, 0.0, 0.0, -max),
            (16, 8.0, 0.0, 0.0, 3.0),
            (17, 0.0, 0.0, 0.0, max),
            (18, 0.0, 0.0, 0.0, 0.0),
            (19, 4.0, 0.0, 0.0, 0.25),
            (20, 0.0, 0.0, 0.0, max),
            (22, 4.0, 0.0, 0.0, 0.5),
            (25, 5.0, 2.0, 0.0, 3.0),
            (26, 5.0, 0.0, 2.0, 3.0),
            (40, 9.0, 0.0, 0.0, 3.0),
            (48, 0.0, 0.0, 0.0, 0.0),
            (49, 0.0, 0.0, 0.0, 1.0),
            (50, 0.0, 0.0, 7.0, 7.0),
        ];
        for &(opcode, a, b, ps, expected) in cases {
            assert_eq!(scalar(opcode, a, b, ps).0, expected, "{} a={} b={} ps={}", SCALAR_OPCODES[opcode as usize].name, a, b, ps);
        }
        let r = scalar(21, -0.0, 0.0, 0.0).0;
        assert!(r == 0.0 && r.is_sign_negative(), "rsqf: -inf becomes -0");
    }

    #[test]
    fn scalar_predicates_kills_and_address_register() {
        let p = |opcode, a| {
            let (r, s) = scalar(opcode, a, 0.0, 0.0);
            (r, s.p0)
        };
        assert_eq!(p(27, 0.0), (0.0, true), "setp_eq");
        assert_eq!(p(27, 2.0), (1.0, false));
        assert_eq!(p(28, 2.0), (0.0, true), "setp_ne");
        assert_eq!(p(29, -1.0), (1.0, false), "setp_gt");
        assert_eq!(p(30, 0.0), (0.0, true), "setp_ge");
        assert_eq!(p(31, 1.0), (0.0, true), "setp_inv");
        assert_eq!(p(31, 0.0), (1.0, false));
        assert_eq!(p(31, 3.0), (3.0, false));
        assert_eq!(p(32, 1.0), (0.0, true), "setp_pop");
        assert_eq!(p(32, 3.0), (2.0, false));
        assert_eq!(p(33, 5.0), (f32::MAX, false), "setp_clr");
        assert_eq!(p(34, 0.0), (0.0, true), "setp_rstr");
        assert_eq!(p(34, 5.0), (5.0, false));
        let (r, s) = scalar(39, 1.0, 0.0, 0.0);
        assert_eq!((r, s.killed), (1.0, true), "kills_one");
        let (r, s) = scalar(35, 2.0, 0.0, 0.0);
        assert_eq!((r, s.killed), (0.0, false), "kills_eq not taken");
        let (r, s) = scalar(23, 2.4, 1.0, 0.0);
        assert_eq!((r, s.a0), (2.4, 2), "maxas: round");
        let (_, s) = scalar(24, 2.9, 1.0, 0.0);
        assert_eq!(s.a0, 2, "maxasf: floor");
    }

    #[test]
    fn previous_scalar_and_predicated_instructions() {
        // adds r5 = 1 + 2 (ps = 3); adds_prev r6 = 1 + ps; setp_eq p0 = (0 == 0); (!p0) add r7; (p0) add r8
        let s_op = |sop, sdst| enc(Alu { vop: VECTOR_MAX, sop, sdst, smask: 0xF, reg: [0, 0, 4], temp: [true; 3], ..Default::default() });
        let pred = |vdst, cond| enc(Alu { vdst, vmask: 0xF, vop: 0, sop: SCALAR_RETAIN_PREV, reg: [2, 2, 0], temp: [true; 3], predicated: true, pred_condition: cond, ..Default::default() });
        let s = run_body(ShaderType::Vertex, &[s_op(0, 5), s_op(1, 6), s_op(27, 9), pred(7, false), pred(8, true)], 0, &mut Mock::new(), |s| {
            s.r[4] = [2.0, 0.0, 0.0, 1.0];
            s.r[2] = [1.0; 4];
        });
        assert_eq!((s.r[5][0], s.r[6][0]), (3.0, 4.0), "adds_prev uses the previous scalar result");
        assert!(!s.p0, "setp_eq(1): p0 = 0");
        assert_eq!((s.r[7], s.r[8]), ([2.0; 4], [0.0; 4]));
    }

    #[test]
    fn constants_negate_abs_clamp_and_relative_addressing() {
        let mut env = Mock::new();
        env.c[7] = [1.0, 2.0, 3.0, 4.0];
        env.c[256 + 7] = [10.0, 20.0, 30.0, 40.0];
        // max r1, c7, c7 (vertex) and (pixel: c7 is constant 263)
        let mov_c7 = enc(Alu { vdst: 1, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [7, 7, 0], ..Default::default() });
        assert_eq!(run_body(ShaderType::Vertex, &[mov_c7], 0, &mut env, |_| {}).r[1], [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(run_body(ShaderType::Pixel, &[mov_c7], 0, &mut env, |_| {}).r[1], [10.0, 20.0, 30.0, 40.0]);
        // add r1, c[5+a0], r0 with a0 = 2 (maxas before it)
        let set_a0 = enc(Alu { vop: VECTOR_MAX, sop: 23, reg: [0, 0, 4], temp: [true; 3], ..Default::default() });
        let rel = enc(Alu { vdst: 1, vmask: 0xF, vop: 0, sop: SCALAR_RETAIN_PREV, reg: [5, 0, 0], temp: [false, true, true], const_0_rel: true, a0_relative: true, ..Default::default() });
        let s = run_body(ShaderType::Vertex, &[set_a0, rel], 0, &mut env, |s| s.r[4] = [0.0, 0.0, 0.0, 2.0]);
        assert_eq!((s.a0, s.r[1]), (2, [1.0, 2.0, 3.0, 4.0]));
        // add_sat r1, -r_abs[2], r3 and a swizzle: r3.wzyx
        let wzyx: u32 = (0..4u32).map(|c| ((3 - c + 4 - c) & 3) << (2 * c)).sum();
        let i = enc(Alu { vdst: 1, vmask: 0xF, vclamp: true, vop: 0, sop: SCALAR_RETAIN_PREV, reg: [2 | 0x80, 3, 0], temp: [true; 3], neg: [true, false, false], swz: [0, wzyx, 0], ..Default::default() });
        let s = run_body(ShaderType::Vertex, &[i], 0, &mut env, |s| {
            s.r[2] = [-0.5, 0.25, -2.0, 0.0];
            s.r[3] = [0.1, 0.2, 3.0, 0.75];
        });
        assert_eq!(s.r[1], [0.25, 1.0, 0.0, 0.1]);
        // abs_constants applies to constants
        let i = enc(Alu { vdst: 1, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [8, 8, 0], abs_constants: true, ..Default::default() });
        env.c[8] = [-1.0, 2.0, -3.0, 0.0];
        assert_eq!(run_body(ShaderType::Vertex, &[i], 0, &mut env, |_| {}).r[1], [1.0, 2.0, 3.0, 0.0]);
    }

    #[test]
    fn scalar_constant_register_forms() {
        // mulsc (opcode 43): c7.w * r1.x (the register index: opcode bit 0 | src3_sel << 1 | swizzle & 0x3C)
        let mut env = Mock::new();
        env.c[7] = [0.0, 0.0, 0.0, 3.0];
        let i = |sop| enc(Alu { vop: VECTOR_MAX, sop, sdst: 5, smask: 0xF, reg: [0, 0, 7], temp: [true, true, false], ..Default::default() });
        let setup = |s: &mut ShaderState| {
            s.r[0] = [2.0, 0.0, 0.0, 0.0];
            s.r[1] = [5.0, 0.0, 0.0, 0.0];
        };
        assert_eq!(run_body(ShaderType::Vertex, &[i(43)], 0, &mut env, setup).r[5][0], 15.0);
        assert_eq!(run_body(ShaderType::Vertex, &[i(44)], 0, &mut env, setup).r[5][0], 5.0, "addsc: c7.w + r0.x");
        assert_eq!(run_body(ShaderType::Vertex, &[i(46)], 0, &mut env, setup).r[5][0], 1.0, "subsc: c7.w - r0.x");
    }

    #[test]
    fn exports_and_constant_components() {
        let mut env = Mock::new();
        // vertex: oPos.x001 (vector x, constant 0 for y z via scalar_dest_rel, constant 1 for w); o3 = r2
        let pos = enc(Alu { vdst: 62, export: true, sdst_rel: true, vmask: 0b1001, smask: 0b1000, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [2, 2, 0], temp: [true; 3], ..Default::default() });
        let o3 = enc(Alu { vdst: 3, export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [2, 2, 0], temp: [true; 3], ..Default::default() });
        let s = run_body(ShaderType::Vertex, &[pos, o3], 0, &mut env, |s| s.r[2] = [5.0, 6.0, 7.0, 8.0]);
        assert_eq!(s.exports.position, Some([5.0, 0.0, 0.0, 1.0]));
        assert_eq!(s.exports.interpolators[3], Some([5.0, 6.0, 7.0, 8.0]));
        assert_eq!(s.exports.interpolators[0], None);
        // pixel: oC0.xyz from the vector op, .w from the scalar op (both in one instruction)
        let c0 = enc(Alu { export: true, vmask: 0b0111, smask: 0b1000, vop: VECTOR_MAX, sop: 5, reg: [2, 2, 4], temp: [true; 3], ..Default::default() });
        let s = run_body(ShaderType::Pixel, &[c0], 0, &mut env, |s| {
            s.r[2] = [0.1, 0.2, 0.3, 0.4];
            s.r[4] = [0.0, 0.0, 0.0, 0.9];
        });
        assert_eq!(s.exports.colors[0], Some([0.1, 0.2, 0.3, 0.9]));
        // memory export is only counted
        let ea = enc(Alu { vdst: 32, export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, temp: [true; 3], ..Default::default() });
        assert_eq!(run_body(ShaderType::Vertex, &[ea], 0, &mut env, |_| {}).exports.memexport_writes, 1);
    }

    #[test]
    fn fetches_write_through_the_destination_swizzle() {
        let mut env = Mock::new();
        // vfetch_full r1.xy01 from r0.y, constant 2 (index 0, sel 2); vfetch_mini r2.x__w offset 4; tfetch2D r3 from r5.zw
        let full = [(1 << 12) | (1 << 19) | (2 << 25) | (1 << 30), (1 << 3) | (4 << 6) | (5 << 9) | (57 << 16), 3];
        let mini = [(2 << 12) | (1 << 19), (7 << 3) | (7 << 6) | (3 << 9) | (1 << 30), 4 << 8];
        let tfetch = [1 | (5 << 5) | (3 << 12) | (1 << 19) | ((2 | (3 << 2)) << 26), 1 << 3 | (2 << 6) | (3 << 9), 1 << 14];
        let s = run_body(ShaderType::Vertex, &[full, mini, tfetch], 0b01_01_01, &mut env, |s| {
            s.r[0] = [0.0, 6.0, 0.0, 0.0];
            s.r[2] = [9.0; 4];
            s.r[5] = [0.0, 0.0, 0.5, 0.75];
        });
        assert_eq!(s.r[1], [60.0, 0.0, 0.0, 1.0]);
        assert_eq!(s.r[2], [60.0, 9.0, 9.0, 3.0], "mini: keep y z, inherits the source and constant");
        assert_eq!(env.vertex_fetches, [(2, 6.0), (2, 6.0)]);
        assert_eq!(s.r[3], [0.25, 0.5, 0.75, 1.0]);
        assert_eq!(env.texture_coords, [[0.5, 0.75, 0.0, 0.0]]);
    }

    #[test]
    fn conditional_exec_uses_the_boolean_constant() {
        // cexece b3 over one ALU writing r1; the shader ends either way
        let body = enc(Alu { vdst: 1, vmask: 0xF, vop: 0, sop: SCALAR_RETAIN_PREV, reg: [2, 2, 0], temp: [true; 3], ..Default::default() });
        let words = shader(&[pack(exec(4, 1, 1, 0, (3 << 2) | (1 << 10)), cf(0, 0, 0)), body]);
        for taken in [false, true] {
            let mut env = Mock::new();
            env.bools[3] = taken;
            let mut s = ShaderState::default();
            s.r[2] = [1.0; 4];
            run(&words, ShaderType::Vertex, &mut s, &mut env).unwrap();
            assert_eq!(s.r[1], if taken { [2.0; 4] } else { [0.0; 4] });
        }
    }

    #[test]
    fn loops_count_and_step_the_loop_register() {
        // 0 loop i0, L3 | 1 exec {add r1, r1, c0; max r[10+aL], c1, c1} | 2 endloop i0, L1 | 3 exece {}
        let add = enc(Alu { vdst: 1, vmask: 0xF, vop: 0, sop: SCALAR_RETAIN_PREV, reg: [1, 0, 0], temp: [true, false, true], ..Default::default() });
        let store = enc(Alu { vdst: 10, vdst_rel: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [1, 1, 0], ..Default::default() });
        let words = shader(&[
            pack(cf(7, 3, 0), exec(EXEC, 2, 2, 0, CLEAN)),
            pack(cf(8, 1, 0), exec(EXEC_END, 2, 0, 0, CLEAN)),
            add,
            store,
        ]);
        let mut env = Mock::new();
        env.c[0] = [1.0; 4];
        env.c[1] = [7.0; 4];
        env.loops[0] = 3 | (5 << 8) | (2 << 16); // 3 iterations, aL = 5, step 2
        let mut s = ShaderState::default();
        run(&words, ShaderType::Vertex, &mut s, &mut env).unwrap();
        assert_eq!(s.r[1], [3.0; 4]);
        assert_eq!([s.r[15], s.r[17], s.r[19], s.r[16]], [[7.0; 4], [7.0; 4], [7.0; 4], [0.0; 4]]);
        assert_eq!(s.al, 0, "the outer aL comes back");
        // a count of 0 skips the body
        env.loops[0] = 0;
        let mut s = ShaderState::default();
        run(&words, ShaderType::Vertex, &mut s, &mut env).unwrap();
        assert_eq!(s.r[1], [0.0; 4]);
    }

    #[test]
    fn calls_returns_jumps_and_the_step_limit() {
        // 0 call L2 | 1 exece {r2 = 1} | 2 exec {r1 = 1} | 3 ret
        let set = |vdst| enc(Alu { vdst, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, reg: [0, 0, 0], ..Default::default() });
        let call = cf(9, 2 | (1 << 13), 0);
        let words = shader(&[pack(call, exec(EXEC_END, 2, 1, 0, CLEAN)), pack(exec(EXEC, 3, 1, 0, CLEAN), cf(10, 0, 0)), set(2), set(1)]);
        let mut env = Mock::new();
        env.c[0] = [1.0; 4];
        let mut s = ShaderState::default();
        run(&words, ShaderType::Vertex, &mut s, &mut env).unwrap();
        assert_eq!((s.r[1], s.r[2]), ([1.0; 4], [1.0; 4]));
        // an unconditional jump to itself never ends
        let words = shader(&[pack(cf(11, 0 | (1 << 13), 0), exec(EXEC_END, 1, 0, 0, CLEAN)), [0; 3]]);
        let err = run(&words, ShaderType::Vertex, &mut ShaderState::default(), &mut env).unwrap_err();
        assert!(err.contains("did not end"), "{}", err);
        // a predicated jump is taken only when p0 matches
        let words = shader(&[pack(cf(11, 2 | (1 << 14) | 0, 1 << 10), exec(EXEC, 2, 1, 0, CLEAN)), pack(exec(EXEC_END, 2, 0, 0, CLEAN), cf(0, 0, 0)), set(1)]);
        let mut s = ShaderState::default();
        run(&words, ShaderType::Vertex, &mut s, &mut env).unwrap();
        assert_eq!(s.r[1], [1.0; 4], "p0 = 0: (p0) jmp not taken");
    }

    /// Runs the shaders dumped with XENON_SHADER_DUMP (guest data outside the repository) on the mock environment:
    /// `XENON_SHADER_DIR=<dir> cargo test --release --lib dumped_shaders -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dumped_shaders_run_to_their_end() {
        let dir = std::env::var("XENON_SHADER_DIR").expect("XENON_SHADER_DIR");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let ty = if name.starts_with("vs_") { ShaderType::Vertex } else if name.starts_with("ps_") { ShaderType::Pixel } else { continue };
            let words: Vec<u32> = std::fs::read(&path).unwrap().chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
            let mut env = Mock::new();
            env.c.iter_mut().enumerate().for_each(|(i, c)| *c = [0.5 + (i % 7) as f32 * 0.1; 4]);
            let mut s = ShaderState::default();
            run(&words, ty, &mut s, &mut env).unwrap_or_else(|e| panic!("{}: {}", name, e));
            println!("{}: position {:?} o0 {:?} oC0 {:?}", name, s.exports.position, s.exports.interpolators[0], s.exports.colors[0]);
            match ty {
                ShaderType::Vertex => assert!(s.exports.position.is_some(), "{}: no position", name),
                ShaderType::Pixel => assert!(s.exports.colors[0].is_some(), "{}: no color", name),
            }
        }
    }

    /// Pixel shader cost: `XENON_SHADER_DIR=<dir> cargo test --release --lib bench_dumped_pixel_shaders -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_dumped_pixel_shaders() {
        let dir = std::env::var("XENON_SHADER_DIR").expect("XENON_SHADER_DIR");
        let mut env = Mock::new();
        env.c.iter_mut().enumerate().for_each(|(i, c)| *c = [0.5 + (i % 7) as f32 * 0.1; 4]);
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            if !name.starts_with("ps_") {
                continue;
            }
            let words: Vec<u32> = std::fs::read(&path).unwrap().chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
            let n = 200_000;
            let start = std::time::Instant::now();
            let mut sum = 0.0f32;
            let compiled = CompiledShader::compile(&words, ShaderType::Pixel, &env);
            for i in 0..n {
                let mut s = ShaderState::default();
                s.r[0] = [i as f32 / n as f32, 0.25, 0.5, 1.0];
                compiled.run(&mut s, &mut env).unwrap();
                sum += s.exports.colors[0].map_or(0.0, |c| c[0]);
            }
            println!("{}: {:.0} ns/pixel (checksum {})", name, start.elapsed().as_nanos() as f64 / n as f64, sum);
        }
    }
}
