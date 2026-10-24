use image::{Rgba, RgbaImage};
use xenon_emu::gpu::{
    build_draw_indexed, build_load_alu_constant, build_set_context_reg, build_wait_for_idle,
    RingBuffer, XenosGpu, PA_CL_VPORT_XOFFSET, PA_CL_VPORT_XSCALE, PA_CL_VPORT_YOFFSET,
    PA_CL_VPORT_YSCALE, XENOS_HEIGHT, XENOS_WIDTH,
};
use xenon_emu::memory::MemoryBus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("  Xenon-Emu: Teste do Pipeline de Shaders ATI Xenos     ");
    println!("========================================================");

    let mut bus = MemoryBus::new();
    let ring_addr: u64 = 0x1000_0000;
    let ring_size: usize = 16 * 1024;
    let mut ring = RingBuffer::new(ring_addr, ring_size);
    let mut gpu = XenosGpu::new();

    println!("[*] 1. Configurando Registradores de Viewport via PM4...");
    let pkt_vport_xs = build_set_context_reg(PA_CL_VPORT_XSCALE, (640.0f32).to_bits());
    let pkt_vport_ys = build_set_context_reg(PA_CL_VPORT_YSCALE, (-360.0f32).to_bits());
    let pkt_vport_xo = build_set_context_reg(PA_CL_VPORT_XOFFSET, (640.0f32).to_bits());
    let pkt_vport_yo = build_set_context_reg(PA_CL_VPORT_YOFFSET, (360.0f32).to_bits());

    ring.write_packet(&mut bus, &pkt_vport_xs)?;
    ring.write_packet(&mut bus, &pkt_vport_ys)?;
    ring.write_packet(&mut bus, &pkt_vport_xo)?;
    ring.write_packet(&mut bus, &pkt_vport_yo)?;

    println!("[*] 2. Carregando Constantes de ALU (c0..c4) via PM4_LOAD_ALU_CONSTANT...");
    // Matrix MVP em c0..c3 (identidade com leve escala e translacao)
    let constants: [[f32; 4]; 5] = [
        [1.0, 0.0, 0.0, 0.0],       // c0: Row 0
        [0.0, 1.0, 0.0, 0.0],       // c1: Row 1
        [0.0, 0.0, 1.0, 0.0],       // c2: Row 2
        [0.0, 0.0, 0.0, 1.0],       // c3: Row 3
        [0.4, 1.2, 0.4, 1.0],       // c4: Diffuse Tint Factor (Green Xbox 360 Boost)
    ];

    let pkt_constants = build_load_alu_constant(0, &constants);
    ring.write_packet(&mut bus, &pkt_constants)?;

    println!("[*] 3. Alocando Indices de Triangulos na Memoria Guest (0x70000000)...");
    let indices_addr: u64 = 0x7000_0000;
    // 2 Triangles for a quad: (0, 1, 2) and (2, 3, 0)
    let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];
    for (i, &idx) in indices.iter().enumerate() {
        let addr = indices_addr + (i * 2) as u64;
        bus.write_u8(addr, (idx >> 8) as u8)?;
        bus.write_u8(addr + 1, (idx & 0xFF) as u8)?;
    }

    println!("[*] 4. Emitindo Pacote PM4_DRAW_INDX...");
    let pkt_draw = build_draw_indexed(0x04, 6, indices_addr as u32);
    ring.write_packet(&mut bus, &pkt_draw)?;

    let pkt_flush = build_wait_for_idle();
    ring.write_packet(&mut bus, &pkt_flush)?;

    println!("[*] 5. Executando Ring Buffer na GPU ATI Xenos...");
    let executed = gpu.execute_ring(&mut bus, &mut ring)?;
    println!("[+] Total de pacotes PM4 executados: {}", executed);
    println!("[+] Total de draw calls: {}", gpu.draw_calls);
    println!("[+] Total de primitivas (triangulos) desenhados: {}", gpu.primitives_drawn);

    assert!(gpu.draw_calls >= 1, "Draw calls deve ser >= 1");
    assert!(gpu.primitives_drawn >= 2, "Primitivas desenhadas deve ser >= 2");

    // Salvar imagem resultante
    let mut img = RgbaImage::new(XENOS_WIDTH as u32, XENOS_HEIGHT as u32);
    for y in 0..XENOS_HEIGHT {
        for x in 0..XENOS_WIDTH {
            let px = gpu.color_buffer[y * XENOS_WIDTH + x];
            let a = ((px >> 24) & 0xFF) as u8;
            let r = ((px >> 16) & 0xFF) as u8;
            let g = ((px >> 8) & 0xFF) as u8;
            let b = (px & 0xFF) as u8;
            img.put_pixel(x as u32, y as u32, Rgba([r, g, b, if a == 0 && (r > 0 || g > 0 || b > 0) { 255 } else { a }]));
        }
    }

    let out_path = "../test_shader_render.png";
    img.save(out_path)?;
    println!("[+] Imagem de teste renderizada e salva em '{}'!", out_path);

    println!("\n========================================================");
    println!("  [+] Pipeline de Shaders ATI Xenos Validado com Sucesso!");
    println!("========================================================");

    Ok(())
}
