use image::{Rgba, RgbaImage};
use xenon_emu::gpu::{
    build_draw_indexed, build_load_alu_constant, build_set_context_reg, build_wait_for_idle,
    RingBuffer, XenosGpu, PA_CL_VPORT_XOFFSET, PA_CL_VPORT_XSCALE, PA_CL_VPORT_YOFFSET,
    PA_CL_VPORT_YSCALE, VGT_DMA_BASE, VGT_DMA_SIZE, VGT_DMA_STRIDE, XENOS_HEIGHT, XENOS_WIDTH,
};
use xenon_emu::memory::MemoryBus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("  Xenon-Emu: Teste de Texturizacao & VGT DMA ATI Xenos  ");
    println!("========================================================");

    let mut bus = MemoryBus::new();
    let ring_addr: u64 = 0x1000_0000;
    let ring_size: usize = 16 * 1024;
    let mut ring = RingBuffer::new(ring_addr, ring_size);
    let mut gpu = XenosGpu::new();

    println!("[*] 1. Carregando Texturas Autenticas da Dashboard via TextureAtlas...");
    gpu.load_atlas_from_dir("../extracted_xui");
    let bound = gpu.bind_texture_by_name(0, "xboxLogo");
    println!("[+] Textura 'xboxLogo' vinculada ao Sampler 0: {}", bound);
    assert!(bound, "Falha ao vincular 'xboxLogo' do diretorio extracted_xui");

    println!("[*] 2. Configurando Viewport registers...");
    let pkt_vport_xs = build_set_context_reg(PA_CL_VPORT_XSCALE, (640.0f32).to_bits());
    let pkt_vport_ys = build_set_context_reg(PA_CL_VPORT_YSCALE, (-360.0f32).to_bits());
    let pkt_vport_xo = build_set_context_reg(PA_CL_VPORT_XOFFSET, (640.0f32).to_bits());
    let pkt_vport_yo = build_set_context_reg(PA_CL_VPORT_YOFFSET, (360.0f32).to_bits());

    ring.write_packet(&mut bus, &pkt_vport_xs)?;
    ring.write_packet(&mut bus, &pkt_vport_ys)?;
    ring.write_packet(&mut bus, &pkt_vport_xo)?;
    ring.write_packet(&mut bus, &pkt_vport_yo)?;

    println!("[*] 3. Alocando Vertex Buffer VGT DMA na Memoria Guest (0x10060000)...");
    let vgt_addr: u64 = 0x1006_0000;
    // 4 Vertices: (x, y, z, color, u, v) em big-endian
    let raw_vertices: [([f32; 3], u32, [f32; 2]); 4] = [
        ([440.0, 160.0, 0.0], 0xFFFF_FFFF, [0.0, 0.0]), // Top-Left
        ([840.0, 160.0, 0.0], 0xFFFF_FFFF, [1.0, 0.0]), // Top-Right
        ([840.0, 560.0, 0.0], 0xFFFF_FFFF, [1.0, 1.0]), // Bottom-Right
        ([440.0, 560.0, 0.0], 0xFFFF_FFFF, [0.0, 1.0]), // Bottom-Left
    ];

    for (i, (pos, color, uv)) in raw_vertices.iter().enumerate() {
        let v_base = vgt_addr + (i * 24) as u64;
        bus.write_u32(v_base, pos[0].to_bits())?;
        bus.write_u32(v_base + 4, pos[1].to_bits())?;
        bus.write_u32(v_base + 8, pos[2].to_bits())?;
        bus.write_u32(v_base + 12, *color)?;
        bus.write_u32(v_base + 16, uv[0].to_bits())?;
        bus.write_u32(v_base + 20, uv[1].to_bits())?;
    }
    println!("  [+] 4 vertices (96 bytes) gravados em 0x{:08X}", vgt_addr);

    println!("[*] 4. Alocando Indices de Triangulos na Memoria Guest (0x10050000)...");
    let indices_addr: u64 = 0x1005_0000;
    let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];
    for (i, &idx) in indices.iter().enumerate() {
        let addr = indices_addr + (i * 2) as u64;
        bus.write_u8(addr, (idx >> 8) as u8)?;
        bus.write_u8(addr + 1, (idx & 0xFF) as u8)?;
    }

    println!("[*] 5. Emitindo Registradores de VGT DMA e Constantes via PM4...");
    let pkt_dma_base = build_set_context_reg(VGT_DMA_BASE, vgt_addr as u32);
    let pkt_dma_size = build_set_context_reg(VGT_DMA_SIZE, 4);
    let pkt_dma_stride = build_set_context_reg(VGT_DMA_STRIDE, 24);

    ring.write_packet(&mut bus, &pkt_dma_base)?;
    ring.write_packet(&mut bus, &pkt_dma_size)?;
    ring.write_packet(&mut bus, &pkt_dma_stride)?;

    let constants: [[f32; 4]; 5] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0], // Full white tint
    ];
    let pkt_constants = build_load_alu_constant(0, &constants);
    ring.write_packet(&mut bus, &pkt_constants)?;

    println!("[*] 6. Emitindo DRAW_INDX e WAIT_FOR_IDLE...");
    let pkt_draw = build_draw_indexed(0x04, 6, indices_addr as u32);
    ring.write_packet(&mut bus, &pkt_draw)?;

    let pkt_flush = build_wait_for_idle();
    ring.write_packet(&mut bus, &pkt_flush)?;

    println!("[*] 7. Executando Ring Buffer na GPU ATI Xenos...");
    let executed = gpu.execute_ring(&mut bus, &mut ring)?;
    println!("[+] Total de pacotes PM4 executados: {}", executed);
    println!("[+] Total de draw calls: {}", gpu.draw_calls);
    println!("[+] Total de primitivas desenhadas: {}", gpu.primitives_drawn);

    assert!(gpu.draw_calls >= 1, "Draw calls deve ser >= 1");
    assert!(gpu.primitives_drawn >= 2, "Primitivas desenhadas deve ser >= 2");

    // Salvar imagem resultante
    let mut img = RgbaImage::new(XENOS_WIDTH as u32, XENOS_HEIGHT as u32);
    let mut non_zero_pixels = 0usize;
    for y in 0..XENOS_HEIGHT {
        for x in 0..XENOS_WIDTH {
            let px = gpu.color_buffer[y * XENOS_WIDTH + x];
            if px != 0 {
                non_zero_pixels += 1;
            }
            let a = ((px >> 24) & 0xFF) as u8;
            let r = ((px >> 16) & 0xFF) as u8;
            let g = ((px >> 8) & 0xFF) as u8;
            let b = (px & 0xFF) as u8;
            img.put_pixel(
                x as u32,
                y as u32,
                Rgba([r, g, b, if a == 0 && (r > 0 || g > 0 || b > 0) { 255 } else { a }]),
            );
        }
    }

    let out_path = "../test_textured_render.png";
    img.save(out_path)?;
    println!("[+] Imagem texturizada salva em '{}' com {} pixels ativos!", out_path, non_zero_pixels);
    assert!(non_zero_pixels > 1000, "Deveria haver pelo menos 1000 pixels renderizados");

    println!("\n========================================================");
    println!("  [+] Pipeline de Texturizacao ATI Xenos Validado com Sucesso!");
    println!("========================================================");

    Ok(())
}
