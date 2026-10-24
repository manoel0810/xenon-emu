//! ETAPA 2: Xenos command processor state and primary ring buffer.

use xenon_emu::gpu::display::resolve_path;
use xenon_emu::gpu::{RingBuffer, XenosGpu};

/// Returns the ring buffer, the GPU and the ring geometry used by the legacy synthetic stream.
pub fn setup(is_metro: bool) -> (RingBuffer, XenosGpu, u64, usize) {
    // ==========================================================
    // ETAPA 2: ATI Xenos GPU Command Processor (CP) & Primary Ring Buffer
    // ==========================================================
    println!("\n>>> [ETAPA 2] ATI Xenos GPU Command Processor (CP) & Primary Ring Buffer <<<");

    let ring_base: u64 = if is_metro { 0x9010_4000 } else { 0x1000_0000 };
    let ring_size: usize = 64 * 1024;  // 64 KB Primary Ring Buffer
    let ring_buffer = RingBuffer::new(ring_base, ring_size);
    let mut xenos_gpu = XenosGpu::new();
    println!("[+] Initialized Xenos Primary Ring Buffer at 0x{:08X} (Size: {} KB)", ring_base, ring_size / 1024);

    // Load authentic Dashboard texture assets into TextureAtlas
    let atlas_dir = resolve_path("extracted_xui");
    xenos_gpu.load_atlas_from_dir(atlas_dir);
    let bound_logo = xenos_gpu.bind_texture_by_name(0, "xboxLogo");
    println!("  [+] Texture 'xboxLogo' bound to Sampler Stage 0: {}", bound_logo);

    (ring_buffer, xenos_gpu, ring_base, ring_size)
}
