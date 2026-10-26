// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2022 Ben Vanik.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/shader_translator.cc         Shader::AnalyzeUcode (control flow walk, labels, exec bodies),
//                                              ParseControlFlow*, Parse{Vertex,Texture}FetchInstruction,
//                                              ParseAluInstruction(Operand), Is{Vector,Scalar}OpDefaultNop
//   src/xenia/gpu/shader_translator_disasm.cc  Disassemble{Result,Source}Operand and Parsed*::Disassemble
//   src/xenia/gpu/shader.h                     SwizzleSource, InstructionResult/Operand, parsed instruction defaults
//
//! Xenos microcode disassembler producing the text of Xenia's ucode disassembly (`Shader::ucode_disassembly`),
//! plus the problems found while decoding (unknown opcodes, exports with no target, exec ranges out of bounds).

use std::collections::BTreeSet;
use std::fmt::Write;

use super::ucode::*;

/// SwizzleSource: 0..3 = X..W, 4 = constant 0, 5 = constant 1.
const SWIZZLE_CHARS: [char; 6] = ['x', 'y', 'z', 'w', '0', '1'];
const SWZ_0: u8 = 4;
const SWZ_1: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Addressing {
    Absolute,
    AddressRegisterRelative,
    LoopRelative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    None,
    Register,
    Interpolator,
    Position,
    PointSizeEdgeFlagKillVertex,
    ExportAddress,
    ExportData,
    Color,
    Depth,
}

/// InstructionResult.
#[derive(Debug, Clone, Copy)]
struct Result {
    target: Target,
    index: u32,
    addressing: Addressing,
    clamped: bool,
    write_mask: u32,
    components: [u8; 4],
}

impl Default for Result {
    fn default() -> Self {
        Self { target: Target::None, index: 0, addressing: Addressing::Absolute, clamped: false, write_mask: 0, components: [0, 1, 2, 3] }
    }
}

/// InstructionOperand (temporary register or float constant).
#[derive(Debug, Clone, Copy)]
struct Operand {
    is_constant: bool,
    index: u32,
    addressing: Addressing,
    negated: bool,
    absolute: bool,
    count: u32,
    components: [u8; 4],
}

impl Operand {
    fn is_standard_swizzle(&self) -> bool {
        self.count == 4 && self.components == [0, 1, 2, 3]
    }
}

fn result_operand(r: &Result, out: &mut String) {
    let indexed = match r.target {
        Target::Register => { out.push('r'); true }
        Target::Interpolator => { out.push('o'); true }
        Target::Position => { out.push_str("oPos"); false }
        Target::PointSizeEdgeFlagKillVertex => { out.push_str("oPts"); false }
        Target::ExportAddress => { out.push_str("eA"); false }
        Target::ExportData => { out.push_str("eM"); true }
        Target::Color => { out.push_str("oC"); true }
        Target::Depth => { out.push_str("oDepth"); false }
        Target::None => false,
    };
    if indexed {
        match r.addressing {
            Addressing::Absolute => { let _ = write!(out, "{}", r.index); }
            Addressing::AddressRegisterRelative => { let _ = write!(out, "[{}+a0]", r.index); }
            Addressing::LoopRelative => { let _ = write!(out, "[{}+aL]", r.index); }
        }
    }
    if r.write_mask == 0 {
        out.push_str("._");
    } else if r.write_mask != 0b1111 || r.components != [0, 1, 2, 3] {
        out.push('.');
        for i in 0..4 {
            out.push(if r.write_mask & (1 << i) != 0 { SWIZZLE_CHARS[r.components[i] as usize] } else { '_' });
        }
    }
}

fn source_operand(op: &Operand, out: &mut String) {
    if op.negated {
        out.push('-');
    }
    out.push(if op.is_constant { 'c' } else { 'r' });
    if op.absolute {
        out.push_str("_abs");
    }
    match op.addressing {
        Addressing::Absolute if op.absolute => { let _ = write!(out, "[{}]", op.index); }
        Addressing::Absolute => { let _ = write!(out, "{}", op.index); }
        Addressing::AddressRegisterRelative => { let _ = write!(out, "[{}+a0]", op.index); }
        Addressing::LoopRelative => { let _ = write!(out, "[{}+aL]", op.index); }
    }
    if !op.is_standard_swizzle() {
        out.push('.');
        for j in 0..op.count as usize {
            out.push(SWIZZLE_CHARS[op.components[j] as usize]);
        }
    }
}

fn predicate_prefix(is_predicated: bool, condition: bool, out: &mut String) {
    out.push_str(if !is_predicated { "      " } else if condition { " (p0) " } else { "(!p0) " });
}

/// The disassembly text and the problems found while decoding.
#[derive(Debug, Default, Clone)]
pub struct Disassembly {
    pub text: String,
    pub issues: Vec<String>,
    /// Number of ALU and fetch instructions decoded from exec bodies.
    pub alu_count: u32,
    pub fetch_count: u32,
}

/// Disassembles a shader (Shader::AnalyzeUcode).
pub fn disassemble(words: &[u32], shader_type: ShaderType) -> Disassembly {
    let mut d = Disassembly::default();
    let pair = |i: usize| unpack_cf_pair([words[i * 3], words[i * 3 + 1], words[i * 3 + 2]]);

    // The control flow program ends at the first exec's address (what freedreno and Xenia do); labels are the
    // targets of calls, jumps and loops.
    // The scan stops at the bound found so far, so ALU/fetch words are not read as control flow.
    let mut bound = words.len() / 3;
    let mut labels = BTreeSet::new();
    let mut i = 0;
    while i < bound {
        for cf in pair(i) {
            if cf.opcode().is_exec() {
                bound = bound.min(cf.exec_address() as usize);
            }
            match cf.opcode() {
                CfOpcode::CondCall | CfOpcode::CondJmp | CfOpcode::LoopStart | CfOpcode::LoopEnd => {
                    labels.insert(cf.target_address());
                }
                _ => {}
            }
        }
        i += 1;
    }

    let mut previous_vfetch_full = FetchInstruction::default();
    let mut ended = false;
    for i in 0..bound {
        for (j, cf) in pair(i).iter().enumerate() {
            let cf_index = (i * 2 + j) as u32;
            if labels.contains(&cf_index) {
                let _ = writeln!(d.text, "                label L{}", cf_index);
            }
            let _ = write!(d.text, "/* {:4}.{} */ ", i, j);
            let out = &mut d.text;
            match cf.opcode() {
                CfOpcode::Nop => out.push_str("      cnop\n"),
                op if op.is_exec() => {
                    ended |= op.ends_shader();
                    exec(&mut d, words, cf, shader_type, &mut previous_vfetch_full);
                }
                CfOpcode::LoopStart => {
                    let _ = write!(out, "      loop i{}, L{}", cf.loop_id(), cf.target_address());
                    if cf.loop_is_repeat() {
                        out.push_str(", Repeat=true");
                    }
                    out.push('\n');
                }
                CfOpcode::LoopEnd => {
                    predicate_prefix(cf.loop_is_predicated_break(), cf.condition(), out);
                    let _ = writeln!(out, "endloop i{}, L{}", cf.loop_id(), cf.target_address());
                }
                CfOpcode::CondCall | CfOpcode::CondJmp => {
                    let (plain, cond) = if cf.opcode() == CfOpcode::CondCall { ("call", "ccall") } else { ("jmp", "cjmp") };
                    if cf.is_unconditional() {
                        let _ = write!(out, "      {} ", plain);
                    } else if cf.is_predicated() {
                        predicate_prefix(true, cf.condition(), out);
                        let _ = write!(out, "{} ", plain);
                    } else {
                        let _ = write!(out, "      {} {}b{}, ", cond, if cf.condition() { "" } else { "!" }, cf.bool_address());
                    }
                    let _ = writeln!(out, "L{}", cf.target_address());
                }
                CfOpcode::Return => out.push_str("      ret\n"),
                CfOpcode::Alloc => {
                    out.push_str("      alloc ");
                    match cf.alloc_type() {
                        AllocType::None => {}
                        AllocType::VsPosition => out.push_str("position"),
                        AllocType::VsInterpolatorsOrPsColors => {
                            out.push_str(if shader_type == ShaderType::Vertex { "interpolators" } else { "colors" })
                        }
                        AllocType::Memory => { let _ = write!(out, "export = {}", cf.alloc_size()); }
                    }
                    out.push('\n');
                }
                // kMarkVsFetchDone prints nothing after the index (as in Xenia)
                _ => {}
            }
        }
    }
    if !ended {
        d.issues.push("no exec ends the shader".into());
    }
    d
}

/// ParsedExecInstruction::Disassemble and Shader::GatherExecInformation (the exec body).
fn exec(d: &mut Disassembly, words: &[u32], cf: &CfInstruction, shader_type: ShaderType, previous_vfetch_full: &mut FetchInstruction) {
    let op = cf.opcode();
    let out = &mut d.text;
    let (conditional, predicated, name, is_predicate_clean) = match op {
        CfOpcode::Exec | CfOpcode::ExecEnd => (false, false, if op == CfOpcode::ExecEnd { "exece" } else { "exec" }, cf.exec_is_predicate_clean()),
        CfOpcode::CondExecPred | CfOpcode::CondExecPredEnd => {
            (false, true, if op == CfOpcode::CondExecPredEnd { "exece" } else { "exec" }, cf.exec_is_predicate_clean())
        }
        // kCondExec*, kCondExecPredClean*: the cond_exec layout; only the plain cond_exec has an unclean predicate
        _ => (true, false, if op.ends_shader() { "cexece" } else { "cexec" }, !matches!(op, CfOpcode::CondExec | CfOpcode::CondExecEnd)),
    };
    if conditional {
        let _ = write!(out, "      {} {}b{}", name, if cf.condition() { "" } else { "!" }, cf.bool_address());
    } else if predicated {
        predicate_prefix(true, cf.condition(), out);
        out.push_str(name);
    } else {
        let _ = write!(out, "      {}", name);
    }
    if cf.exec_is_yield() {
        if conditional {
            out.push(',');
        }
        out.push_str(" Yield=true");
    }
    if !is_predicate_clean {
        out.push_str("    // PredicateClean=false");
    }
    out.push('\n');

    let mut sequence = cf.exec_sequence();
    let address = cf.exec_address();
    for offset in address..address + cf.exec_count() {
        let _ = write!(d.text, "/* {:4}   */ ", offset);
        if sequence & 0b10 != 0 {
            d.text.push_str("         serialize\n             ");
        }
        let base = offset as usize * 3;
        let Some(slice) = words.get(base..base + 3) else {
            d.issues.push(format!("exec instruction {} past the end of the shader ({} dwords)", offset, words.len()));
            d.text.push_str("<out of range>\n");
            sequence >>= 2;
            continue;
        };
        let w = [slice[0], slice[1], slice[2]];
        if sequence & 0b01 != 0 {
            d.fetch_count += 1;
            let f = FetchInstruction(w);
            if f.opcode() == FetchOpcode::VertexFetch {
                vertex_fetch(&mut d.text, &f, previous_vfetch_full);
            } else {
                texture_fetch(d, &f);
            }
        } else {
            d.alu_count += 1;
            alu(d, &AluInstruction(w), shader_type);
        }
        sequence >>= 2;
    }
}

/// ParseFetchInstructionResult.
fn fetch_result(dest: u32, swizzle: u32, relative: bool) -> Result {
    let mut r = Result {
        target: Target::Register,
        index: dest,
        addressing: if relative { Addressing::LoopRelative } else { Addressing::Absolute },
        write_mask: 0b1111,
        ..Default::default()
    };
    for i in 0..4 {
        r.components[i] = match fetch_dest_component_swizzle(swizzle, i as u32) {
            c @ 0..=3 => c as u8,
            5 => SWZ_1,
            7 => {
                r.write_mask &= !(1 << i);
                SWZ_0
            }
            // 4 (constant 0) or the invalid swizzle 6
            _ => SWZ_0,
        };
    }
    r
}

/// ParseVertexFetchInstruction + ParsedVertexFetchInstruction::Disassemble. A mini fetch reuses the source,
/// constant, stride and index rounding of the previous full fetch.
fn vertex_fetch(out: &mut String, op: &FetchInstruction, previous_full: &mut FetchInstruction) {
    let mini = op.vf_is_mini_fetch();
    let full = if mini { *previous_full } else { *op };
    if !mini {
        *previous_full = *op;
    }
    out.push_str("   ");
    predicate_prefix(op.is_predicated(), op.predicate_condition(), out);
    out.push_str(if mini { "vfetch_mini " } else { "vfetch_full " });
    result_operand(&fetch_result(op.dest(), op.dest_swizzle(), op.is_dest_relative()), out);
    if !mini {
        out.push_str(", ");
        let src = Operand {
            is_constant: false,
            index: full.src(),
            addressing: if full.is_src_relative() { Addressing::LoopRelative } else { Addressing::Absolute },
            negated: false,
            absolute: false,
            count: 1,
            components: [(full.vf_src_swizzle() & 3) as u8, 0, 0, 0],
        };
        source_operand(&src, out);
        let _ = write!(out, ", vf{}", 95 - full.vf_fetch_constant_index() as i32);
        if full.vf_is_index_rounded() {
            out.push_str(", RoundIndex=true");
        }
    }
    if op.vf_exp_adjust() != 0 {
        let _ = write!(out, ", ExpAdjust={}", op.vf_exp_adjust());
    }
    if op.vf_offset() != 0 {
        let _ = write!(out, ", Offset={}", op.vf_offset());
    }
    let format = op.vf_data_format();
    if format != 0 {
        match vertex_format_name(format) {
            Some(name) => { let _ = write!(out, ", DataFormat={}", name); }
            None => { let _ = write!(out, ", DataFormat=<{}>", format); }
        }
    }
    if !mini && full.vf_stride() != 0 {
        let _ = write!(out, ", Stride={}", full.vf_stride());
    }
    if op.vf_is_signed() {
        out.push_str(", Signed=true");
    }
    if !op.vf_is_normalized() {
        out.push_str(", NumFormat=integer");
    }
    if op.vf_prefetch_count() != 0 {
        let _ = write!(out, ", PrefetchCount={}", op.vf_prefetch_count() + 1);
    }
    out.push('\n');
}

/// ParseTextureFetchInstruction + ParsedTextureFetchInstruction::Disassemble.
fn texture_fetch(d: &mut Disassembly, op: &FetchInstruction) {
    const DIM: [&str; 4] = ["1D", "2D", "3D", "Cube"];
    const FILTERS: [&str; 4] = ["point", "linear", "basemap", "keep"];
    const ANISO: [&str; 7] = ["disabled", "max1to1", "max2to1", "max4to1", "max8to1", "max16to1", "keep"];
    let dim = op.tf_dimension();
    // name, has dest, has fetch constant, has attributes, source component count override
    let (name, has_dest, has_const, has_attributes, override_count) = match op.opcode() {
        FetchOpcode::TextureFetch => (format!("tfetch{}", DIM[dim as usize]), true, true, true, 0),
        FetchOpcode::GetTextureBorderColorFrac => (format!("getBCF{}", DIM[dim as usize]), true, true, true, 0),
        FetchOpcode::GetTextureComputedLod => (format!("getCompTexLOD{}", DIM[dim as usize]), true, true, true, 0),
        FetchOpcode::GetTextureGradients => ("getGradients".into(), true, true, true, 2),
        FetchOpcode::GetTextureWeights => (format!("getWeights{}", DIM[dim as usize]), true, true, true, 0),
        FetchOpcode::SetTextureLod => ("setTexLOD".into(), false, false, false, 1),
        FetchOpcode::SetTextureGradientsHorz => ("setGradientH".into(), false, false, false, 3),
        FetchOpcode::SetTextureGradientsVert => ("setGradientV".into(), false, false, false, 3),
        FetchOpcode::VertexFetch | FetchOpcode::Unknown(_) => {
            let FetchOpcode::Unknown(v) = op.opcode() else { unreachable!() };
            d.issues.push(format!("unknown fetch opcode {}", v));
            let _ = writeln!(d.text, "         <fetch opcode {}: {:08X} {:08X} {:08X}>", v, op.0[0], op.0[1], op.0[2]);
            return;
        }
    };
    let out = &mut d.text;
    out.push_str("   ");
    predicate_prefix(op.is_predicated(), op.predicate_condition(), out);
    out.push_str(&name);
    out.push(' ');
    if has_dest {
        result_operand(&fetch_result(op.dest(), op.dest_swizzle(), op.is_dest_relative()), out);
        out.push_str(", ");
    }
    let mut count = if override_count != 0 { override_count } else { fetch_dimension_component_count(dim) };
    let swizzle = op.tf_src_swizzle();
    if op.opcode() == FetchOpcode::TextureFetch && dim == 0 {
        // a 1D fetch may provide XY for a 2D fetch constant
        let x = swizzle & 3;
        if (swizzle >> 2) & 3 != x || (swizzle >> 4) & 3 != x {
            count = 2;
        }
    }
    let mut src = Operand {
        is_constant: false,
        index: op.src(),
        addressing: if op.is_src_relative() { Addressing::LoopRelative } else { Addressing::Absolute },
        negated: false,
        absolute: false,
        count,
        components: [0; 4],
    };
    for j in 0..count as usize {
        src.components[j] = ((swizzle >> (2 * j)) & 3) as u8;
    }
    source_operand(&src, out);
    if has_const {
        let _ = write!(out, ", tf{}", op.tf_fetch_constant_index());
    }
    if has_attributes {
        if !op.tf_fetch_valid_only() {
            out.push_str(", FetchValidOnly=false");
        }
        if op.tf_unnormalized_coordinates() {
            out.push_str(", UnnormalizedTextureCoords=true");
        }
        for (label, value) in [("MagFilter", op.tf_mag_filter()), ("MinFilter", op.tf_min_filter()), ("MipFilter", op.tf_mip_filter())] {
            if value != 3 {
                let _ = write!(out, ", {}={}", label, FILTERS[value as usize]);
            }
        }
        if op.tf_aniso_filter() != 7 {
            let _ = write!(out, ", AnisoFilter={}", ANISO[op.tf_aniso_filter() as usize]);
        }
        for (label, value) in [("VolMagFilter", op.tf_vol_mag_filter()), ("VolMinFilter", op.tf_vol_min_filter())] {
            if value != 3 {
                let _ = write!(out, ", {}={}", label, FILTERS[value as usize]);
            }
        }
        if !op.tf_use_computed_lod() {
            out.push_str(", UseComputedLOD=false");
        }
        if op.tf_use_register_lod() {
            out.push_str(", UseRegisterLOD=true");
        }
        if op.tf_use_register_gradients() {
            out.push_str(", UseRegisterGradients=true");
        }
        if op.tf_lod_bias() != 0.0 {
            let _ = write!(out, ", LODBias={}", op.tf_lod_bias());
        }
        let components = fetch_dimension_component_count(dim);
        if op.tf_offset_x() != 0.0 {
            let _ = write!(out, ", OffsetX={}", op.tf_offset_x());
        }
        if components > 1 && op.tf_offset_y() != 0.0 {
            let _ = write!(out, ", OffsetY={}", op.tf_offset_y());
        }
        if components > 2 && op.tf_offset_z() != 0.0 {
            let _ = write!(out, ", OffsetZ={}", op.tf_offset_z());
        }
    }
    out.push('\n');
}

/// ParseAluInstructionOperand: `count` components (1 = scalar W; 2 = W and `second`; 4 = vector).
fn alu_operand(op: &AluInstruction, i: u32, count: u32, second: u32) -> Operand {
    let reg = op.src_reg(i);
    let swizzle = op.src_swizzle(i);
    let mut o = Operand { is_constant: false, index: 0, addressing: Addressing::Absolute, negated: op.src_negate(i), absolute: false, count, components: [0; 4] };
    if op.src_is_temp(i) {
        o.index = reg & 0x3F;
        o.absolute = reg & 0x80 != 0;
        if reg & 0x40 != 0 {
            o.addressing = Addressing::LoopRelative;
        }
    } else {
        o.is_constant = true;
        o.index = reg;
        o.absolute = op.abs_constants();
        if op.src_const_is_addressed(i) {
            o.addressing = if op.is_const_address_register_relative() { Addressing::AddressRegisterRelative } else { Addressing::LoopRelative };
        }
    }
    match count {
        1 => o.components[0] = swizzled_component_index(swizzle, 3) as u8,
        2 => {
            o.components[0] = swizzled_component_index(swizzle, 3) as u8;
            o.components[1] = swizzled_component_index(swizzle, second) as u8;
        }
        _ => {
            for j in 0..4 {
                o.components[j] = swizzled_component_index(swizzle, j as u32) as u8;
            }
        }
    }
    o
}

/// ParseAluInstruction + ParsedAluInstruction::Disassemble.
fn alu(d: &mut Disassembly, op: &AluInstruction, shader_type: ShaderType) {
    // Both operations of an export write to the export register in vector_dest.
    let mut target = Target::Register;
    let mut export_index = 0;
    if op.is_export() {
        let reg = op.vector_dest();
        target = match reg {
            export::EXPORT_ADDRESS => Target::ExportAddress,
            export::EXPORT_DATA_0..=export::EXPORT_DATA_4 => {
                export_index = reg - export::EXPORT_DATA_0;
                Target::ExportData
            }
            export::VS_INTERPOLATOR_0..=export::VS_INTERPOLATOR_15 if shader_type == ShaderType::Vertex => {
                export_index = reg;
                Target::Interpolator
            }
            export::VS_POSITION if shader_type == ShaderType::Vertex => Target::Position,
            export::VS_POINT_SIZE_EDGE_FLAG_KILL_VERTEX if shader_type == ShaderType::Vertex => Target::PointSizeEdgeFlagKillVertex,
            export::PS_COLOR_0..=export::PS_COLOR_3 if shader_type == ShaderType::Pixel => {
                export_index = reg;
                Target::Color
            }
            export::PS_DEPTH if shader_type == ShaderType::Pixel => Target::Depth,
            _ => {
                d.issues.push(format!("unsupported write to export {}", reg));
                Target::None
            }
        };
    }

    // vector operation and constant 0/1 writes
    let vector_opcode = op.vector_opcode();
    let vinfo = &VECTOR_OPCODES[vector_opcode as usize];
    let mut vresult = Result { target, clamped: op.vector_clamp(), ..Default::default() };
    if op.is_export() {
        vresult.index = export_index;
    } else {
        vresult.index = op.vector_dest();
        if op.is_vector_dest_relative() {
            vresult.addressing = Addressing::LoopRelative;
        }
    }
    let (c0, c1) = (op.constant_0_write_mask(), op.constant_1_write_mask());
    vresult.write_mask = op.vector_op_result_write_mask() | c0 | c1;
    for i in 0..4 {
        vresult.components[i] = if c0 & (1 << i) != 0 { SWZ_0 } else if c1 & (1 << i) != 0 { SWZ_1 } else { i as u8 };
    }
    let vector_operands: Vec<Operand> = (0..vinfo.operand_count()).map(|i| alu_operand(op, i + 1, 4, 0)).collect();

    // scalar operation
    let scalar_opcode = op.scalar_opcode();
    let sinfo = &SCALAR_OPCODES[scalar_opcode as usize];
    let mut sresult = Result { target, clamped: op.scalar_clamp(), write_mask: op.scalar_op_result_write_mask(), ..Default::default() };
    if op.is_export() {
        sresult.index = export_index;
    } else {
        sresult.index = op.scalar_dest();
        if op.is_scalar_dest_relative() {
            sresult.addressing = Addressing::LoopRelative;
        }
    }
    let scalar_operands: Vec<Operand> = match sinfo.operand_count {
        0 => Vec::new(),
        // shares source 3 with the vector operation: `b` is Z if the vector operation uses source 3, else X
        1 => vec![alu_operand(op, 3, if sinfo.single_operand_is_two_component { 2 } else { 1 }, if vinfo.operand_count() == 3 { 2 } else { 0 })],
        // constant (W) and temporary register (X) sharing source 3's negate and swizzle
        _ => {
            let swizzle = op.src_swizzle(3);
            let mut c = Operand {
                is_constant: true,
                index: op.src_reg(3),
                addressing: Addressing::Absolute,
                negated: op.src_negate(3),
                absolute: op.abs_constants(),
                count: 1,
                components: [swizzled_component_index(swizzle, 3) as u8, 0, 0, 0],
            };
            if op.src_const_is_addressed(3) {
                c.addressing = if op.is_const_address_register_relative() { Addressing::AddressRegisterRelative } else { Addressing::LoopRelative };
            }
            let r = Operand {
                is_constant: false,
                index: op.scalar_const_reg_op_src_temp_reg(),
                components: [swizzled_component_index(swizzle, 0) as u8, 0, 0, 0],
                ..c
            };
            vec![c, r]
        }
    };

    // IsVectorOpDefaultNop / IsScalarOpDefaultNop
    let scalar_nop = scalar_opcode == SCALAR_RETAIN_PREV
        && sresult.write_mask == 0
        && !sresult.clamped
        && (sresult.target != Target::Register || (sresult.index == 0 && sresult.addressing == Addressing::Absolute));
    let is_r0 = |o: &Operand| !o.is_constant && o.index == 0 && o.addressing == Addressing::Absolute && !o.negated && !o.absolute && o.is_standard_swizzle();
    let vector_nop = vector_opcode == VECTOR_MAX
        && vresult.write_mask == 0
        && !vresult.clamped
        && vector_operands.iter().all(is_r0)
        && if vresult.target == Target::Register { vresult.index == 0 && vresult.addressing == Addressing::Absolute } else { !scalar_nop };

    let out = &mut d.text;
    if vector_nop && scalar_nop {
        out.push_str("   ");
        predicate_prefix(op.is_predicated(), op.predicate_condition(), out);
        out.push_str("nop\n");
        return;
    }
    if !vector_nop {
        out.push_str("   ");
        predicate_prefix(op.is_predicated(), op.predicate_condition(), out);
        out.push_str(vinfo.name);
        if vresult.clamped {
            out.push_str("_sat");
        }
        out.push(' ');
        result_operand(&vresult, out);
        for o in &vector_operands {
            out.push_str(", ");
            source_operand(o, out);
        }
        out.push('\n');
    }
    if !scalar_nop {
        out.push_str(if vector_nop { "   " } else { "              + " });
        predicate_prefix(op.is_predicated(), op.predicate_condition(), out);
        out.push_str(sinfo.name);
        if sresult.clamped {
            out.push_str("_sat");
        }
        out.push(' ');
        result_operand(&sresult, out);
        for o in &scalar_operands {
            out.push_str(", ");
            source_operand(o, out);
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A control flow instruction as (dword_0, 16-bit dword_1).
    type Cf = (u32, u32);

    fn cf(opcode: u32, dw0: u32, dw1_low12: u32) -> Cf {
        (dw0, (opcode << 12) | dw1_low12)
    }
    fn exec(opcode: u32, address: u32, count: u32, sequence: u32, dw1_low12: u32) -> Cf {
        cf(opcode, address | (count << 12) | (sequence << 16), dw1_low12)
    }
    fn alloc(kind: u32, size: u32) -> Cf {
        cf(12, size, kind << 9)
    }
    /// The inverse of unpack_cf_pair.
    fn pack(a: Cf, b: Cf) -> [u32; 3] {
        [a.0, (a.1 & 0xFFFF) | (b.0 << 16), (b.0 >> 16) | (b.1 << 16)]
    }

    #[derive(Default, Clone, Copy)]
    struct Alu {
        vdst: u32,
        sdst: u32,
        sdst_rel: bool,
        export: bool,
        vmask: u32,
        smask: u32,
        vclamp: bool,
        sop: u32,
        vop: u32,
        swz: [u32; 3],
        neg: [bool; 3],
        reg: [u32; 3],
        temp: [bool; 3],
        const_0_rel: bool,
        a0_relative: bool,
        predicated: bool,
        pred_condition: bool,
    }

    fn alu(a: Alu) -> [u32; 3] {
        let b = |v: bool| v as u32;
        [
            a.vdst | a.sdst << 8 | b(a.sdst_rel) << 14 | b(a.export) << 15 | a.vmask << 16 | a.smask << 20 | b(a.vclamp) << 24 | a.sop << 26,
            a.swz[2] | a.swz[1] << 8 | a.swz[0] << 16 | b(a.neg[2]) << 24 | b(a.neg[1]) << 25 | b(a.neg[0]) << 26
                | b(a.pred_condition) << 27 | b(a.predicated) << 28 | b(a.a0_relative) << 29 | b(a.const_0_rel) << 31,
            a.reg[2] | a.reg[1] << 8 | a.reg[0] << 16 | a.vop << 24 | b(a.temp[2]) << 29 | b(a.temp[1]) << 30 | b(a.temp[0]) << 31,
        ]
    }

    /// Vector max r0._, r0, r0 with retain_prev: the default nop.
    fn alu_nop() -> [u32; 3] {
        alu(Alu { vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, temp: [true; 3], ..Default::default() })
    }

    fn shader(parts: &[[u32; 3]]) -> Vec<u32> {
        parts.iter().flatten().copied().collect()
    }

    fn lines(d: &Disassembly) -> Vec<&str> {
        d.text.lines().collect()
    }

    #[test]
    fn control_flow_pairs_unpack() {
        let [x, y] = unpack_cf_pair(pack(exec(2, 1, 1, 0, 1 << 9), alloc(2, 0)));
        assert_eq!((x.opcode(), x.exec_address(), x.exec_count()), (CfOpcode::ExecEnd, 1, 1));
        assert_eq!((y.opcode(), y.alloc_type()), (CfOpcode::Alloc, AllocType::VsInterpolatorsOrPsColors));
        assert!(x.opcode().ends_shader() && !y.opcode().is_exec());
    }

    #[test]
    fn pixel_shader_exporting_a_register_to_color() {
        let mov = alu(Alu { export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, temp: [true; 3], ..Default::default() });
        let d = disassemble(&shader(&[pack(alloc(2, 0), exec(2, 1, 1, 0, 1 << 9)), mov]), ShaderType::Pixel);
        assert!(d.issues.is_empty());
        assert_eq!(lines(&d), ["/*    0.0 */       alloc colors", "/*    0.1 */       exece", "/*    1   */          max oC0, r0, r0"]);
        assert_eq!((d.alu_count, d.fetch_count), (1, 0));
    }

    #[test]
    fn alu_operands_swizzles_and_scalar_forms() {
        // swizzle fields are relative to the component: component c reads ((field + c) & 3)
        let yzwx: u32 = (0..4).map(|c| 1 << (2 * c)).sum();
        let xxxx: u32 = (0..4).map(|c| ((4 - c) & 3) << (2 * c)).sum();
        let a = Alu {
            vdst: 2,
            vmask: 0b0011,
            vclamp: true,
            vop: 11, // mad
            sdst: 7,
            smask: 0b1000,
            sop: 43, // mulsc: temporary register = (opcode & 1) | src3_sel << 1 | (src3 swizzle & 0x3C) = r1
            swz: [yzwx, xxxx, 0],
            neg: [true, false, false],
            reg: [1, 5, 9],
            temp: [true, false, false],
            ..Default::default()
        };
        let d = disassemble(&shader(&[pack(exec(2, 1, 1, 0, 1 << 9), cf(0, 0, 0)), alu(a)]), ShaderType::Vertex);
        assert_eq!(lines(&d)[1..3], ["/*    1   */          mad_sat r2.xy__, -r1.yzwx, c5.xxxx, c9", "              +       mulsc r7.___w, c9.w, r1.x"]);
        // absolute temporary, address-register-relative constant, predicated on !p0
        let b = Alu {
            vdst: 1,
            vmask: 0xF,
            sop: SCALAR_RETAIN_PREV,
            reg: [3 | 0x80, 20, 0],
            temp: [true, false, true],
            const_0_rel: true,
            a0_relative: true,
            predicated: true,
            ..Default::default()
        };
        let d = disassemble(&shader(&[pack(exec(2, 1, 1, 0, 1 << 9), cf(0, 0, 0)), alu(b)]), ShaderType::Vertex);
        assert_eq!(lines(&d)[1], "/*    1   */    (!p0) add r1, r_abs[3], c[20+a0]");
    }

    #[test]
    fn exports_with_constant_components_and_nops() {
        // scalar_dest_rel on an export writes 0 to the unmasked components, both masks write 1
        let a = Alu { vdst: export::VS_POSITION, export: true, sdst_rel: true, vmask: 0b1001, smask: 0b1000, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, temp: [true; 3], ..Default::default() };
        let d = disassemble(&shader(&[pack(alloc(1, 0), exec(2, 1, 2, 0, 1 << 9)), alu(a), alu_nop()]), ShaderType::Vertex);
        assert_eq!(lines(&d), ["/*    0.0 */       alloc position", "/*    0.1 */       exece", "/*    1   */          max oPos.x001, r0, r0", "/*    2   */          nop"]);
        // a position export means nothing to a pixel shader
        let bad = Alu { vdst: export::VS_POSITION, export: true, vmask: 0xF, vop: VECTOR_MAX, sop: SCALAR_RETAIN_PREV, temp: [true; 3], ..Default::default() };
        let d = disassemble(&shader(&[pack(exec(2, 1, 1, 0, 1 << 9), cf(0, 0, 0)), alu(bad)]), ShaderType::Pixel);
        assert_eq!(d.issues, ["unsupported write to export 62"]);
    }

    #[test]
    fn vertex_fetches_full_and_mini() {
        // full: r1.xyz1 from r0.x, constant 0, FMT_32_32_32_FLOAT, stride 7; mini: r2, offset 3, FMT_8_8_8_8
        let full = [(1 << 12) | (1 << 19), (1 << 3) | (2 << 6) | (5 << 9) | (57 << 16), 7];
        let mini = [(2 << 12) | (1 << 19), (1 << 3) | (2 << 6) | (3 << 9) | (6 << 16) | (1 << 30), 3 << 8];
        let d = disassemble(&shader(&[pack(exec(2, 1, 2, 0b0101, 1 << 9), cf(0, 0, 0)), full, mini]), ShaderType::Vertex);
        assert!(d.issues.is_empty());
        assert_eq!(lines(&d)[1..3], [
            "/*    1   */          vfetch_full r1.xyz1, r0.x, vf95, DataFormat=FMT_32_32_32_FLOAT, Stride=7",
            "/*    2   */          vfetch_mini r2, Offset=3, DataFormat=FMT_8_8_8_8",
        ]);
        assert_eq!(d.fetch_count, 2);
    }

    #[test]
    fn texture_fetch_attributes() {
        // tfetch2D r3.xy__ from r1.zw, constant 4; mag linear, min point, the rest from the fetch constant; bias -0.5
        let t = [
            1 | (1 << 5) | (3 << 12) | (1 << 19) | (4 << 20) | ((2 | (3 << 2)) << 26),
            (1 << 3) | (7 << 6) | (7 << 9) | (1 << 12) | (3 << 16) | (7 << 18) | (3 << 24) | (3 << 26) | (1 << 28),
            (((-8i32) as u32 & 0x7F) << 2) | (1 << 14),
        ];
        let d = disassemble(&shader(&[pack(exec(2, 1, 1, 0b01, 1 << 9), cf(0, 0, 0)), t]), ShaderType::Pixel);
        assert_eq!(lines(&d)[1], "/*    1   */          tfetch2D r3.xy__, r1.zw, tf4, MagFilter=linear, MinFilter=point, LODBias=-0.5");
        let d = disassemble(&shader(&[pack(exec(2, 1, 1, 0b01, 1 << 9), cf(0, 0, 0)), [5, 0, 0]]), ShaderType::Pixel);
        assert_eq!(d.issues, ["unknown fetch opcode 5"]);
    }

    #[test]
    fn conditional_flow_loops_and_labels() {
        let loop_start = cf(7, 3 | (2 << 16), 0); // loop i2, L3
        let cexec = exec(3, 4, 1, 0, 7 << 2); // cexec !b7 (plain cond_exec: predicate not clean)
        let pexec = exec(5, 4, 1, 0, (1 << 10) | (1 << 9)); // (p0) exec
        let pexec = (pexec.0 | (1 << 15), pexec.1); // ... yield
        let loop_end = cf(8, 1 | (2 << 16), 0); // endloop i2, L1
        let ccall = cf(9, 5, (3 << 2) | (1 << 10)); // ccall b3, L5
        let pjmp = cf(11, 4 | (1 << 14), 0); // (!p0) jmp L4
        let exece = exec(2, 4, 1, 0, 1 << 9);
        let ret = cf(10, 0, 0);
        let words = shader(&[pack(loop_start, cexec), pack(pexec, loop_end), pack(ccall, pjmp), pack(exece, ret), alu_nop()]);
        let d = disassemble(&words, ShaderType::Vertex);
        assert!(d.issues.is_empty(), "{:?}", d.issues);
        assert_eq!(lines(&d), [
            "/*    0.0 */       loop i2, L3",
            "                label L1",
            "/*    0.1 */       cexec !b7    // PredicateClean=false",
            "/*    4   */          nop",
            "/*    1.0 */  (p0) exec Yield=true",
            "/*    4   */          nop",
            "                label L3",
            "/*    1.1 */       endloop i2, L1",
            "                label L4",
            "/*    2.0 */       ccall b3, L5",
            "                label L5",
            "/*    2.1 */ (!p0) jmp L4",
            "/*    3.0 */       exece",
            "/*    4   */          nop",
            "/*    3.1 */       ret",
        ]);
    }

    #[test]
    fn exec_past_the_end_and_missing_end_are_reported() {
        let d = disassemble(&shader(&[pack(exec(1, 1, 2, 0, 0), cf(0, 0, 0)), alu_nop()]), ShaderType::Vertex);
        assert_eq!(d.issues, ["exec instruction 2 past the end of the shader (6 dwords)", "no exec ends the shader"]);
    }

    /// Disassembles the shaders dumped with XENON_SHADER_DUMP (guest data, kept outside the repository):
    /// `XENON_SHADER_DIR=<dir> cargo test --release --lib dumped_shaders -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dumped_shaders_decode_without_issues() {
        let dir = std::env::var("XENON_SHADER_DIR").expect("XENON_SHADER_DIR");
        let mut count = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let ty = if name.starts_with("vs_") { ShaderType::Vertex } else if name.starts_with("ps_") { ShaderType::Pixel } else { continue };
            let bytes = std::fs::read(&path).unwrap();
            let words: Vec<u32> = bytes.chunks_exact(4).map(|c| u32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect();
            let d = disassemble(&words, ty);
            println!("{}: {} ALU, {} fetch", name, d.alu_count, d.fetch_count);
            assert!(d.issues.is_empty(), "{}: {:?}\n{}", name, d.issues, d.text);
            assert!(d.alu_count > 0, "{}: no ALU instruction", name);
            count += 1;
        }
        assert!(count > 0, "no shaders in {}", dir);
    }
}
