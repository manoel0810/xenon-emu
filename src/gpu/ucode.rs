// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2022 Ben Vanik.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/ucode.h      control flow, fetch and ALU instruction layouts, opcodes, export registers
//   src/xenia/gpu/ucode.cc     kAluScalarOpcodeInfos / kAluVectorOpcodeInfos
//   src/xenia/gpu/xenos.h      FetchOpDimension, TextureFilter, AnisoFilter, VertexFormat values
//
//! Xenos shader microcode. A shader is a list of big-endian dwords: control flow instructions come first, two per
//! three dwords; the exec instructions point at ALU / fetch instructions of three dwords each. The layouts are the
//! C++ bitfields of ucode.h (allocated from the least significant bit).

/// `n` bits of `w` starting at bit `lo`.
#[inline]
fn bits(w: u32, lo: u32, n: u32) -> u32 {
    (w >> lo) & ((1u32 << n) - 1)
}

/// `n` bits of `w` starting at bit `lo`, sign-extended.
#[inline]
fn sbits(w: u32, lo: u32, n: u32) -> i32 {
    ((w << (32 - lo - n)) as i32) >> (32 - n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShaderType {
    Vertex,
    Pixel,
}

// ---------------------------------------------------------------------------------------------------------------
// Control flow
// ---------------------------------------------------------------------------------------------------------------

/// ControlFlowOpcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfOpcode {
    Nop = 0,
    Exec = 1,
    ExecEnd = 2,
    CondExec = 3,
    CondExecEnd = 4,
    CondExecPred = 5,
    CondExecPredEnd = 6,
    LoopStart = 7,
    LoopEnd = 8,
    CondCall = 9,
    Return = 10,
    CondJmp = 11,
    Alloc = 12,
    CondExecPredClean = 13,
    CondExecPredCleanEnd = 14,
    MarkVsFetchDone = 15,
}

impl CfOpcode {
    fn from_bits(v: u32) -> Self {
        use CfOpcode::*;
        [Nop, Exec, ExecEnd, CondExec, CondExecEnd, CondExecPred, CondExecPredEnd, LoopStart, LoopEnd, CondCall, Return,
            CondJmp, Alloc, CondExecPredClean, CondExecPredCleanEnd, MarkVsFetchDone][(v & 0xF) as usize]
    }

    /// IsControlFlowOpcodeExec.
    pub fn is_exec(self) -> bool {
        use CfOpcode::*;
        matches!(self, Exec | ExecEnd | CondExec | CondExecEnd | CondExecPred | CondExecPredEnd | CondExecPredClean | CondExecPredCleanEnd)
    }

    /// DoesControlFlowOpcodeEndShader.
    pub fn ends_shader(self) -> bool {
        use CfOpcode::*;
        matches!(self, ExecEnd | CondExecEnd | CondExecPredEnd | CondExecPredCleanEnd)
    }
}

/// AllocType (kVsInterpolators and kPsColors share 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllocType {
    None = 0,
    VsPosition = 1,
    VsInterpolatorsOrPsColors = 2,
    Memory = 3,
}

/// One 48-bit control flow instruction, as the two dwords of ControlFlowInstruction (dword_1 holds 16 bits). The
/// accessors read the fields of the variant matching the opcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CfInstruction {
    pub dword_0: u32,
    pub dword_1: u32,
}

impl CfInstruction {
    pub fn opcode(&self) -> CfOpcode {
        CfOpcode::from_bits(bits(self.dword_1, 12, 4))
    }
    /// AddressingMode: true = absolute.
    pub fn is_address_absolute(&self) -> bool {
        bits(self.dword_1, 11, 1) == 1
    }

    // exec / cond_exec / cond_exec_pred
    pub fn exec_address(&self) -> u32 {
        bits(self.dword_0, 0, 12)
    }
    pub fn exec_count(&self) -> u32 {
        bits(self.dword_0, 12, 3)
    }
    pub fn exec_is_yield(&self) -> bool {
        bits(self.dword_0, 15, 1) == 1
    }
    /// Two bits per instruction: bit 0 = fetch (else ALU), bit 1 = serialize.
    pub fn exec_sequence(&self) -> u32 {
        bits(self.dword_0, 16, 12)
    }
    /// exec, cond_exec_pred.
    pub fn exec_is_predicate_clean(&self) -> bool {
        bits(self.dword_1, 9, 1) == 1
    }
    /// cond_exec, cond_call, cond_jmp.
    pub fn bool_address(&self) -> u32 {
        bits(self.dword_1, 2, 8)
    }
    /// cond_exec, cond_exec_pred, loop_end, cond_call, cond_jmp.
    pub fn condition(&self) -> bool {
        bits(self.dword_1, 10, 1) == 1
    }

    // loop_start / loop_end / cond_call / cond_jmp
    pub fn target_address(&self) -> u32 {
        bits(self.dword_0, 0, 13)
    }
    pub fn loop_is_repeat(&self) -> bool {
        bits(self.dword_0, 13, 1) == 1
    }
    pub fn loop_id(&self) -> u32 {
        bits(self.dword_0, 16, 5)
    }
    pub fn loop_is_predicated_break(&self) -> bool {
        bits(self.dword_0, 21, 1) == 1
    }
    pub fn is_unconditional(&self) -> bool {
        bits(self.dword_0, 13, 1) == 1
    }
    pub fn is_predicated(&self) -> bool {
        bits(self.dword_0, 14, 1) == 1
    }

    // alloc
    pub fn alloc_size(&self) -> u32 {
        bits(self.dword_0, 0, 3)
    }
    pub fn alloc_type(&self) -> AllocType {
        [AllocType::None, AllocType::VsPosition, AllocType::VsInterpolatorsOrPsColors, AllocType::Memory][bits(self.dword_1, 9, 2) as usize]
    }
}

/// UnpackControlFlowInstructions: the two instructions packed in three dwords.
pub fn unpack_cf_pair(d: [u32; 3]) -> [CfInstruction; 2] {
    [
        CfInstruction { dword_0: d[0], dword_1: d[1] & 0xFFFF },
        CfInstruction { dword_0: (d[1] >> 16) | (d[2] << 16), dword_1: d[2] >> 16 },
    ]
}

// ---------------------------------------------------------------------------------------------------------------
// Fetch
// ---------------------------------------------------------------------------------------------------------------

/// FetchOpcode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchOpcode {
    VertexFetch,
    TextureFetch,
    GetTextureBorderColorFrac,
    GetTextureComputedLod,
    GetTextureGradients,
    GetTextureWeights,
    SetTextureLod,
    SetTextureGradientsHorz,
    SetTextureGradientsVert,
    Unknown(u32),
}

impl FetchOpcode {
    fn from_bits(v: u32) -> Self {
        use FetchOpcode::*;
        match v {
            0 => VertexFetch,
            1 => TextureFetch,
            16 => GetTextureBorderColorFrac,
            17 => GetTextureComputedLod,
            18 => GetTextureGradients,
            19 => GetTextureWeights,
            24 => SetTextureLod,
            25 => SetTextureGradientsHorz,
            26 => SetTextureGradientsVert,
            v => Unknown(v),
        }
    }
}

/// FetchDestinationSwizzle of one destination component: 0..3 = X..W, 4 = 0, 5 = 1, 7 = keep.
pub fn fetch_dest_component_swizzle(swizzle: u32, component: u32) -> u32 {
    (swizzle >> (3 * component)) & 0b111
}

/// A fetch instruction (three dwords); vertex and texture fetches share the common fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FetchInstruction(pub [u32; 3]);

impl FetchInstruction {
    pub fn opcode(&self) -> FetchOpcode {
        FetchOpcode::from_bits(bits(self.0[0], 0, 5))
    }
    pub fn src(&self) -> u32 {
        bits(self.0[0], 5, 6)
    }
    pub fn is_src_relative(&self) -> bool {
        bits(self.0[0], 11, 1) == 1
    }
    pub fn dest(&self) -> u32 {
        bits(self.0[0], 12, 6)
    }
    pub fn is_dest_relative(&self) -> bool {
        bits(self.0[0], 18, 1) == 1
    }
    pub fn dest_swizzle(&self) -> u32 {
        bits(self.0[1], 0, 12)
    }
    pub fn is_predicated(&self) -> bool {
        bits(self.0[1], 31, 1) == 1
    }
    pub fn predicate_condition(&self) -> bool {
        bits(self.0[2], 31, 1) == 1
    }

    // VertexFetchInstruction
    pub fn vf_fetch_constant_index(&self) -> u32 {
        bits(self.0[0], 20, 5) * 3 + bits(self.0[0], 25, 2)
    }
    pub fn vf_prefetch_count(&self) -> u32 {
        bits(self.0[0], 27, 3)
    }
    pub fn vf_src_swizzle(&self) -> u32 {
        bits(self.0[0], 30, 2)
    }
    pub fn vf_is_signed(&self) -> bool {
        bits(self.0[1], 12, 1) == 1
    }
    pub fn vf_is_normalized(&self) -> bool {
        bits(self.0[1], 13, 1) == 0
    }
    pub fn vf_signed_rf_mode(&self) -> u32 {
        bits(self.0[1], 14, 1)
    }
    pub fn vf_is_index_rounded(&self) -> bool {
        bits(self.0[1], 15, 1) == 1
    }
    /// xenos::VertexFormat.
    pub fn vf_data_format(&self) -> u32 {
        bits(self.0[1], 16, 6)
    }
    pub fn vf_exp_adjust(&self) -> i32 {
        sbits(self.0[1], 24, 6)
    }
    pub fn vf_is_mini_fetch(&self) -> bool {
        bits(self.0[1], 30, 1) == 1
    }
    pub fn vf_stride(&self) -> u32 {
        bits(self.0[2], 0, 8)
    }
    pub fn vf_offset(&self) -> i32 {
        sbits(self.0[2], 8, 23)
    }

    // TextureFetchInstruction
    pub fn tf_fetch_constant_index(&self) -> u32 {
        bits(self.0[0], 20, 5)
    }
    pub fn tf_fetch_valid_only(&self) -> bool {
        bits(self.0[0], 19, 1) == 1
    }
    pub fn tf_unnormalized_coordinates(&self) -> bool {
        bits(self.0[0], 25, 1) == 1
    }
    pub fn tf_src_swizzle(&self) -> u32 {
        bits(self.0[0], 26, 6)
    }
    /// xenos::TextureFilter (3 = use the fetch constant).
    pub fn tf_mag_filter(&self) -> u32 {
        bits(self.0[1], 12, 2)
    }
    pub fn tf_min_filter(&self) -> u32 {
        bits(self.0[1], 14, 2)
    }
    pub fn tf_mip_filter(&self) -> u32 {
        bits(self.0[1], 16, 2)
    }
    /// xenos::AnisoFilter (7 = use the fetch constant).
    pub fn tf_aniso_filter(&self) -> u32 {
        bits(self.0[1], 18, 3)
    }
    pub fn tf_vol_mag_filter(&self) -> u32 {
        bits(self.0[1], 24, 2)
    }
    pub fn tf_vol_min_filter(&self) -> u32 {
        bits(self.0[1], 26, 2)
    }
    pub fn tf_use_computed_lod(&self) -> bool {
        bits(self.0[1], 28, 1) == 1
    }
    pub fn tf_use_register_lod(&self) -> bool {
        bits(self.0[1], 29, 1) == 1
    }
    pub fn tf_use_register_gradients(&self) -> bool {
        bits(self.0[2], 0, 1) == 1
    }
    pub fn tf_sample_location(&self) -> u32 {
        bits(self.0[2], 1, 1)
    }
    pub fn tf_lod_bias(&self) -> f32 {
        sbits(self.0[2], 2, 7) as f32 * (1.0 / 16.0)
    }
    /// xenos::FetchOpDimension: 0 1D, 1 2D, 2 3D or stacked, 3 cube.
    pub fn tf_dimension(&self) -> u32 {
        bits(self.0[2], 14, 2)
    }
    pub fn tf_offset_x(&self) -> f32 {
        sbits(self.0[2], 16, 5) as f32 * 0.5
    }
    pub fn tf_offset_y(&self) -> f32 {
        sbits(self.0[2], 21, 5) as f32 * 0.5
    }
    pub fn tf_offset_z(&self) -> f32 {
        sbits(self.0[2], 26, 5) as f32 * 0.5
    }
}

/// GetFetchOpDimensionComponentCount.
pub fn fetch_dimension_component_count(dimension: u32) -> u32 {
    match dimension {
        0 => 1,
        1 => 2,
        _ => 3,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// ALU
// ---------------------------------------------------------------------------------------------------------------

/// AluOpChangedState.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangedState {
    None,
    AddressRegister,
    Predicate,
    PixelKill,
}

/// AluScalarOpcodeInfo: name, operand count (0, 1 = one or two components of src3, 2 = c#.w and r#.x), whether a
/// single operand uses two components, state changed.
pub struct ScalarOpcodeInfo {
    pub name: &'static str,
    pub operand_count: u32,
    pub single_operand_is_two_component: bool,
    pub changed_state: ChangedState,
}

const fn s(name: &'static str, operand_count: u32, two: bool, changed_state: ChangedState) -> ScalarOpcodeInfo {
    ScalarOpcodeInfo { name, operand_count, single_operand_is_two_component: two, changed_state }
}

use ChangedState as C;

/// kAluScalarOpcodeInfos (6 opcode bits).
pub const SCALAR_OPCODES: [ScalarOpcodeInfo; 64] = [
    s("adds", 1, true, C::None),
    s("adds_prev", 1, false, C::None),
    s("muls", 1, true, C::None),
    s("muls_prev", 1, false, C::None),
    s("muls_prev2", 1, true, C::None),
    s("maxs", 1, true, C::None),
    s("mins", 1, true, C::None),
    s("seqs", 1, false, C::None),
    s("sgts", 1, false, C::None),
    s("sges", 1, false, C::None),
    s("snes", 1, false, C::None),
    s("frcs", 1, false, C::None),
    s("truncs", 1, false, C::None),
    s("floors", 1, false, C::None),
    s("exp", 1, false, C::None),
    s("logc", 1, false, C::None),
    s("log", 1, false, C::None),
    s("rcpc", 1, false, C::None),
    s("rcpf", 1, false, C::None),
    s("rcp", 1, false, C::None),
    s("rsqc", 1, false, C::None),
    s("rsqf", 1, false, C::None),
    s("rsq", 1, false, C::None),
    s("maxas", 1, true, C::AddressRegister),
    s("maxasf", 1, true, C::AddressRegister),
    s("subs", 1, true, C::None),
    s("subs_prev", 1, false, C::None),
    s("setp_eq", 1, false, C::Predicate),
    s("setp_ne", 1, false, C::Predicate),
    s("setp_gt", 1, false, C::Predicate),
    s("setp_ge", 1, false, C::Predicate),
    s("setp_inv", 1, false, C::Predicate),
    s("setp_pop", 1, false, C::Predicate),
    s("setp_clr", 0, false, C::Predicate),
    s("setp_rstr", 1, false, C::Predicate),
    s("kills_eq", 1, false, C::PixelKill),
    s("kills_gt", 1, false, C::PixelKill),
    s("kills_ge", 1, false, C::PixelKill),
    s("kills_ne", 1, false, C::PixelKill),
    s("kills_one", 1, false, C::PixelKill),
    s("sqrt", 1, false, C::None),
    s("opcode_41", 0, false, C::None),
    s("mulsc", 2, false, C::None),
    s("mulsc", 2, false, C::None),
    s("addsc", 2, false, C::None),
    s("addsc", 2, false, C::None),
    s("subsc", 2, false, C::None),
    s("subsc", 2, false, C::None),
    s("sin", 1, false, C::None),
    s("cos", 1, false, C::None),
    s("retain_prev", 0, false, C::None),
    s("opcode_51", 0, false, C::None),
    s("opcode_52", 0, false, C::None),
    s("opcode_53", 0, false, C::None),
    s("opcode_54", 0, false, C::None),
    s("opcode_55", 0, false, C::None),
    s("opcode_56", 0, false, C::None),
    s("opcode_57", 0, false, C::None),
    s("opcode_58", 0, false, C::None),
    s("opcode_59", 0, false, C::None),
    s("opcode_60", 0, false, C::None),
    s("opcode_61", 0, false, C::None),
    s("opcode_62", 0, false, C::None),
    s("opcode_63", 0, false, C::None),
];

/// AluScalarOpcode::kRetainPrev.
pub const SCALAR_RETAIN_PREV: u32 = 50;

/// AluVectorOpcodeInfo: name, components used of each source (0 = no such operand), state changed.
pub struct VectorOpcodeInfo {
    pub name: &'static str,
    pub operand_components_used: [u32; 3],
    pub changed_state: ChangedState,
}

impl VectorOpcodeInfo {
    pub fn operand_count(&self) -> u32 {
        self.operand_components_used.iter().take_while(|&&c| c != 0).count() as u32
    }
}

const fn v(name: &'static str, operand_components_used: [u32; 3], changed_state: ChangedState) -> VectorOpcodeInfo {
    VectorOpcodeInfo { name, operand_components_used, changed_state }
}

/// kAluVectorOpcodeInfos (5 opcode bits).
pub const VECTOR_OPCODES: [VectorOpcodeInfo; 32] = [
    v("add", [0b1111, 0b1111, 0], C::None),
    v("mul", [0b1111, 0b1111, 0], C::None),
    v("max", [0b1111, 0b1111, 0], C::None),
    v("min", [0b1111, 0b1111, 0], C::None),
    v("seq", [0b1111, 0b1111, 0], C::None),
    v("sgt", [0b1111, 0b1111, 0], C::None),
    v("sge", [0b1111, 0b1111, 0], C::None),
    v("sne", [0b1111, 0b1111, 0], C::None),
    v("frc", [0b1111, 0, 0], C::None),
    v("trunc", [0b1111, 0, 0], C::None),
    v("floor", [0b1111, 0, 0], C::None),
    v("mad", [0b1111, 0b1111, 0b1111], C::None),
    v("cndeq", [0b1111, 0b1111, 0b1111], C::None),
    v("cndge", [0b1111, 0b1111, 0b1111], C::None),
    v("cndgt", [0b1111, 0b1111, 0b1111], C::None),
    v("dp4", [0b1111, 0b1111, 0], C::None),
    v("dp3", [0b0111, 0b0111, 0], C::None),
    v("dp2add", [0b0011, 0b0011, 0b0001], C::None),
    v("cube", [0b1111, 0b1111, 0], C::None),
    v("max4", [0b1111, 0, 0], C::None),
    v("setp_eq_push", [0b1001, 0b1001, 0], C::Predicate),
    v("setp_ne_push", [0b1001, 0b1001, 0], C::Predicate),
    v("setp_gt_push", [0b1001, 0b1001, 0], C::Predicate),
    v("setp_ge_push", [0b1001, 0b1001, 0], C::Predicate),
    v("kill_eq", [0b1111, 0b1111, 0], C::PixelKill),
    v("kill_gt", [0b1111, 0b1111, 0], C::PixelKill),
    v("kill_ge", [0b1111, 0b1111, 0], C::PixelKill),
    v("kill_ne", [0b1111, 0b1111, 0], C::PixelKill),
    v("dst", [0b0110, 0b1010, 0], C::None),
    v("maxa", [0b1111, 0b1111, 0], C::AddressRegister),
    v("opcode_30", [0, 0, 0], C::None),
    v("opcode_31", [0, 0, 0], C::None),
];

/// AluVectorOpcode::kMax.
pub const VECTOR_MAX: u32 = 2;

/// ExportRegister values.
pub mod export {
    pub const VS_INTERPOLATOR_0: u32 = 0;
    pub const VS_INTERPOLATOR_15: u32 = 15;
    pub const VS_POSITION: u32 = 62;
    pub const VS_POINT_SIZE_EDGE_FLAG_KILL_VERTEX: u32 = 63;
    pub const PS_COLOR_0: u32 = 0;
    pub const PS_COLOR_3: u32 = 3;
    pub const PS_DEPTH: u32 = 61;
    pub const EXPORT_ADDRESS: u32 = 32;
    pub const EXPORT_DATA_0: u32 = 33;
    pub const EXPORT_DATA_4: u32 = 37;
}

/// An ALU instruction (three dwords): a vector and a scalar operation sharing up to three sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AluInstruction(pub [u32; 3]);

impl AluInstruction {
    pub fn vector_dest(&self) -> u32 {
        bits(self.0[0], 0, 6)
    }
    pub fn is_vector_dest_relative(&self) -> bool {
        bits(self.0[0], 6, 1) == 1
    }
    pub fn abs_constants(&self) -> bool {
        bits(self.0[0], 7, 1) == 1
    }
    pub fn scalar_dest(&self) -> u32 {
        bits(self.0[0], 8, 6)
    }
    pub fn is_scalar_dest_relative(&self) -> bool {
        bits(self.0[0], 14, 1) == 1
    }
    pub fn is_export(&self) -> bool {
        bits(self.0[0], 15, 1) == 1
    }
    pub fn vector_write_mask(&self) -> u32 {
        bits(self.0[0], 16, 4)
    }
    pub fn scalar_write_mask(&self) -> u32 {
        bits(self.0[0], 20, 4)
    }
    pub fn vector_clamp(&self) -> bool {
        bits(self.0[0], 24, 1) == 1
    }
    pub fn scalar_clamp(&self) -> bool {
        bits(self.0[0], 25, 1) == 1
    }
    pub fn scalar_opcode(&self) -> u32 {
        bits(self.0[0], 26, 6)
    }
    /// Source `i` (1..=3) swizzle: 2 bits per component, relative to the component index.
    pub fn src_swizzle(&self, i: u32) -> u32 {
        bits(self.0[1], 8 * (3 - i), 8)
    }
    pub fn src_negate(&self, i: u32) -> bool {
        bits(self.0[1], 24 + (3 - i), 1) == 1
    }
    pub fn predicate_condition(&self) -> bool {
        bits(self.0[1], 27, 1) == 1
    }
    pub fn is_predicated(&self) -> bool {
        bits(self.0[1], 28, 1) == 1
    }
    pub fn is_const_address_register_relative(&self) -> bool {
        bits(self.0[1], 29, 1) == 1
    }
    fn const_1_rel_abs(&self) -> bool {
        bits(self.0[1], 30, 1) == 1
    }
    fn const_0_rel_abs(&self) -> bool {
        bits(self.0[1], 31, 1) == 1
    }
    pub fn src_reg(&self, i: u32) -> u32 {
        bits(self.0[2], 8 * (3 - i), 8)
    }
    pub fn vector_opcode(&self) -> u32 {
        bits(self.0[2], 24, 5)
    }
    /// Source `i` is a temporary register (else a float constant).
    pub fn src_is_temp(&self, i: u32) -> bool {
        bits(self.0[2], 29 + (3 - i), 1) == 1
    }
    /// The constant of source `i` is dynamically addressed: the first constant source uses const_0_rel_abs, the
    /// second const_1_rel_abs.
    pub fn src_const_is_addressed(&self, i: u32) -> bool {
        match i {
            1 => self.const_0_rel_abs(),
            2 => if self.src_is_temp(1) { self.const_0_rel_abs() } else { self.const_1_rel_abs() },
            _ => if self.src_is_temp(1) && self.src_is_temp(2) { self.const_0_rel_abs() } else { self.const_1_rel_abs() },
        }
    }
    /// The temporary register of the scalar constant-register operations (mulsc/addsc/subsc).
    pub fn scalar_const_reg_op_src_temp_reg(&self) -> u32 {
        (self.scalar_opcode() & 1) | ((self.src_is_temp(3) as u32) << 1) | (self.src_swizzle(3) & 0x3C)
    }
    pub fn vector_op_result_write_mask(&self) -> u32 {
        let mask = self.vector_write_mask();
        if self.is_export() { mask & !self.scalar_write_mask() } else { mask }
    }
    pub fn scalar_op_result_write_mask(&self) -> u32 {
        let mask = self.scalar_write_mask();
        if self.is_export() { mask & !self.vector_write_mask() } else { mask }
    }
    /// Export components written with constant 0 (scalar_dest_rel on an export) and 1 (both masks set).
    pub fn constant_0_write_mask(&self) -> u32 {
        if !self.is_export() || !self.is_scalar_dest_relative() {
            return 0;
        }
        0b1111 & !(self.vector_write_mask() | self.scalar_write_mask())
    }
    pub fn constant_1_write_mask(&self) -> u32 {
        if !self.is_export() {
            return 0;
        }
        self.vector_write_mask() & self.scalar_write_mask()
    }
}

/// AluInstruction::GetSwizzledComponentIndex: ALU swizzles are relative to the component index.
pub fn swizzled_component_index(swizzle: u32, component_index: u32) -> u32 {
    ((swizzle >> (2 * component_index)) + component_index) & 3
}

/// Vertex formats with a name in Xenia's disassembler (xenos::VertexFormat).
pub fn vertex_format_name(format: u32) -> Option<&'static str> {
    Some(match format {
        6 => "FMT_8_8_8_8",
        7 => "FMT_2_10_10_10",
        16 => "FMT_10_11_11",
        17 => "FMT_11_11_10",
        25 => "FMT_16_16",
        26 => "FMT_16_16_16_16",
        31 => "FMT_16_16_FLOAT",
        32 => "FMT_16_16_16_16_FLOAT",
        33 => "FMT_32",
        34 => "FMT_32_32",
        35 => "FMT_32_32_32_32",
        36 => "FMT_32_FLOAT",
        37 => "FMT_32_32_FLOAT",
        38 => "FMT_32_32_32_32_FLOAT",
        57 => "FMT_32_32_32_FLOAT",
        _ => return None,
    })
}
