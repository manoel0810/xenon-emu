pub mod display;
pub mod font;
pub mod pm4;
pub mod register_file;
pub mod rasterizer;
pub mod ring_buffer;
pub mod shader;
pub mod texture;
pub mod xenos;

pub use display::{
    BufferSource, ColorSwizzle, DisplayMode, GpuDisplay, UntileMode, ViewMode, DISPLAY_HEIGHT,
    DISPLAY_WIDTH,
};
pub use pm4::{
    build_draw_indexed, build_load_alu_constant, build_set_constant, build_set_context_reg,
    build_set_shader_program, build_type0, build_type3, build_wait_for_idle, Pm4Packet,
    PA_CL_VPORT_XOFFSET, PA_CL_VPORT_XSCALE, PA_CL_VPORT_YOFFSET, PA_CL_VPORT_YSCALE,
    PA_SC_WINDOW_OFFSET, RB_COLOR_INFO, SQ_PROGRAM_CNTL, SQ_PS_PROGRAM_ADDR, SQ_VS_PROGRAM_ADDR,
    VGT_DMA_BASE, VGT_DMA_SIZE, VGT_DMA_STRIDE,
};
pub use rasterizer::{
    blend_pixel, rasterize_gradient_quad, rasterize_rounded_card, rasterize_triangle,
    rasterize_triangle_textured,
};
pub use ring_buffer::RingBuffer;
pub use shader::{
    create_default_xui_pixel_shader, create_default_xui_vertex_shader, AluInstruction, AluOpcode,
    FetchOpcode, PixelShader, ShaderReg, VertexInput, VertexOutput, VertexShader,
};
pub use texture::{Texture, TextureAtlas};
pub use xenos::{Vertex, XenosGpu, XENOS_HEIGHT, XENOS_WIDTH};

