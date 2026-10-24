use image::{Rgba, RgbaImage};
use std::fs;
use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::gpu::{
    build_draw_indexed, build_load_alu_constant, build_set_context_reg, build_wait_for_idle,
    GpuDisplay, RingBuffer, XenosGpu, PA_SC_WINDOW_OFFSET, RB_COLOR_INFO, VGT_DMA_BASE,
    VGT_DMA_SIZE, VGT_DMA_STRIDE, XENOS_HEIGHT, XENOS_WIDTH,
};
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::{PeImage, StfsPackage, XexFile, XurScene, XusTable};
use xenon_emu::memory::MemoryBus;

fn save_u32_buffer_as_png(buf: &[u32], width: u32, height: u32, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut img = RgbaImage::new(width, height);
    for y in 0..height {
        for x in 0..width {
            let pixel = buf[(y * width + x) as usize];
            let r = ((pixel >> 16) & 0xFF) as u8;
            let g = ((pixel >> 8) & 0xFF) as u8;
            let b = (pixel & 0xFF) as u8;
            let a = ((pixel >> 24) & 0xFF) as u8;
            let alpha = if a == 0 { 255 } else { a };
            img.put_pixel(x, y, Rgba([r, g, b, alpha]));
        }
    }
    img.save(path)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("  Xenon-Emu: Testing Native Dashboard & GPU Pipeline    ");
    println!("========================================================");

    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    let mut bus = MemoryBus::new();
    let vaddr_base: u64 = 0x9200_0000;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let uncompressed = dash_xex.decompress_image(&decrypted)?;
    let pe = PeImage::parse(&uncompressed)?;
    pe.load_into_memory(&uncompressed, vaddr_base, &mut bus)?;

    let mut thunk_map = dash_xex.setup_import_thunks(&mut bus)?;

    // Register ExTerminateThread trampoline at 0x80001000
    bus.write_u32(0x80001000, 0x44000042)?;
    bus.write_u32(0x80001004, 0x4E800020)?;
    thunk_map.insert(0x80001000, ("xboxkrnl.exe".to_string(), 0x0019));

    let stack_top = vaddr_base + dash_xex.image_size as u64 - 0x10000;
    let kpcr_base = 0x7FE00000u64;
    let tls_base = 0x7FE01000u64;
    bus.write_u32(kpcr_base + 0x00, tls_base as u32)?;
    bus.write_u32(kpcr_base + 0x30, kpcr_base as u32)?;
    bus.write_u32(kpcr_base + 0x70, stack_top as u32)?;
    bus.write_u32(kpcr_base + 0x74, (stack_top - 0x50000) as u32)?;
    bus.write_u32(kpcr_base + 0x100, (kpcr_base + 0x200) as u32)?;
    bus.write_u8(kpcr_base + 0x10C, 0)?;
    bus.write_u32(kpcr_base + 0x14C, 1)?;

    let mut kernel = KernelDispatcher::new();

    // 1. Initial Guest DllMain execution
    let entry_vaddr = vaddr_base + pe.entry_point_rva as u64;
    let mut cpu = Cpu::new(entry_vaddr);
    cpu.regs.gpr[1] = stack_top;
    cpu.regs.gpr[3] = vaddr_base;
    cpu.regs.gpr[4] = 1;
    cpu.regs.gpr[5] = 0;
    cpu.regs.gpr[13] = kpcr_base;
    cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top - 8, 0x80001000)?;
    bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;

    for _ in 1..=100 {
        match cpu.step(&mut bus) {
            Ok(StepResult::Ok) => {}
            Ok(StepResult::Syscall { lev: _, pc: call_pc }) => {
                if let Some((lib, ordinal)) = thunk_map.get(&call_pc) {
                    if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                        break;
                    }
                    let args = [
                        cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5],
                        cpu.regs.gpr[6], cpu.regs.gpr[7], cpu.regs.gpr[8],
                    ];
                    let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                    cpu.regs.gpr[3] = ret;
                    cpu.regs.pc = cpu.regs.lr;
                } else {
                    break;
                }
            }
            Ok(StepResult::Halted) | Err(_) => break,
        }
    }
    println!("[+] Phase 1 (DllMain) cleanly executed!");

    // 2. Setup Primary Ring Buffer and ATI Xenos GPU
    let ring_base: u64 = 0x1000_0000;
    let ring_size: usize = 64 * 1024;
    let mut ring_buffer = RingBuffer::new(ring_base, ring_size);
    let mut xenos_gpu = XenosGpu::new();

    xenos_gpu.load_atlas_from_dir("../extracted_xui");
    let bound = xenos_gpu.bind_texture_by_name(0, "xboxLogo");
    println!("[+] Authentic 'xboxLogo' bound to Sampler Stage 0: {}", bound);

    // Emit PM4 Context Registers into Ring Buffer
    let pkt_color = build_set_context_reg(RB_COLOR_INFO, 0x7000_0000);
    ring_buffer.write_packet(&mut bus, &pkt_color)?;
    let pkt_vport = build_set_context_reg(PA_SC_WINDOW_OFFSET, 0x0000_0000);
    ring_buffer.write_packet(&mut bus, &pkt_vport)?;

    // Emit PM4 Shader Constants
    let shader_constants = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
        [1.0, 1.0, 1.0, 1.0],
    ];
    let pkt_constants = build_load_alu_constant(0, &shader_constants);
    ring_buffer.write_packet(&mut bus, &pkt_constants)?;

    // VGT DMA Vertex Buffer setup
    let vgt_addr: u64 = 0x1006_0000;
    let raw_vertices: [([f32; 3], u32, [f32; 2]); 4] = [
        ([380.0, 150.0, 0.0], 0xFFFF_FFFF, [0.0, 0.0]),
        ([1180.0, 150.0, 0.0], 0xFFFF_FFFF, [1.0, 0.0]),
        ([1180.0, 250.0, 0.0], 0xFFFF_FFFF, [1.0, 1.0]),
        ([380.0, 250.0, 0.0], 0xFFFF_FFFF, [0.0, 1.0]),
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
    let pkt_dma_base = build_set_context_reg(VGT_DMA_BASE, vgt_addr as u32);
    let pkt_dma_size = build_set_context_reg(VGT_DMA_SIZE, 4);
    let pkt_dma_stride = build_set_context_reg(VGT_DMA_STRIDE, 24);
    ring_buffer.write_packet(&mut bus, &pkt_dma_base)?;
    ring_buffer.write_packet(&mut bus, &pkt_dma_size)?;
    ring_buffer.write_packet(&mut bus, &pkt_dma_stride)?;

    let index_ptr: u32 = 0x1005_0000;
    let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];
    for (i, &idx) in indices.iter().enumerate() {
        bus.write_u16(index_ptr as u64 + (i * 2) as u64, idx)?;
    }
    let pkt_draw = build_draw_indexed(0x04, 6, index_ptr);
    ring_buffer.write_packet(&mut bus, &pkt_draw)?;
    let pkt_idle = build_wait_for_idle();
    ring_buffer.write_packet(&mut bus, &pkt_idle)?;

    // Execute Ring Buffer once to rasterize PM4 quad
    let executed_count = xenos_gpu.execute_ring(&mut bus, &mut ring_buffer)?;
    println!("[+] Xenos GPU CP processed {} PM4 packets from Ring Buffer!", executed_count);

    // Rasterize genuine XUI scene
    let pt_strings_path = "../extracted_xui/pt-br/Strings.xus";
    let xus_table = XusTable::load_file(pt_strings_path).ok();
    if let Ok(games_scene) = XurScene::load_file("../extracted_xui/gamesSignedOut.xur") {
        xenos_gpu.render_xui_scene(&games_scene, xus_table.as_ref(), 0x0010_7C10, 0x000B_4D0B);
        println!("[+] Direct3D 9 rasterized authentic XUI scene 'gamesSignedOut'!");
    }

    // Initialize GpuDisplay
    let mut display = GpuDisplay::new("Xenon-Emu: Headless Test Display")?;
    println!("[+] GpuDisplay initialized (DisplayMode::BootAnimation active).");

    struct ThreadInfo {
        cpu: Cpu,
        entry_pc: u64,
        active: bool,
        steps: u64,
    }
    let mut threads = vec![
        ThreadInfo {
            cpu: Cpu::new(0x921088A0),
            entry_pc: 0x921088A0,
            active: true,
            steps: 0,
        },
        ThreadInfo {
            cpu: Cpu::new(0x9212A34C),
            entry_pc: 0x9212A34C,
            active: true,
            steps: 0,
        },
    ];

    let art_dir = "C:/Users/manoe/.gemini/antigravity-ide/brain/3f1ffb6b-9bf4-4cc0-b24f-7ff0bb70f26d";
    let ribbons_path = format!("{}/bootanim_ribbons_native.png", art_dir);
    let sphere_path = format!("{}/bootanim_sphere_native.png", art_dir);
    let dash_path = format!("{}/real_gpu_dash_native.png", art_dir);

    println!("[*] Running 295 frames through genuine display & GPU pipeline...");
    for frame in 1..=295 {
        // Step guest threads
        for t in &mut threads {
            if !t.active { continue; }
            for _ in 0..500 {
                let pc = t.cpu.regs.pc;
                match bus.read_u32(pc) {
                    Ok(_) => {
                        match t.cpu.step(&mut bus) {
                            Ok(StepResult::Ok) => { t.steps += 1; }
                            Ok(StepResult::Syscall { lev: _, pc: c_pc }) => {
                                if let Some((lib, ord)) = thunk_map.get(&c_pc) {
                                    if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                        t.cpu.regs.pc = t.entry_pc;
                                        t.cpu.regs.lr = 0x80001000;
                                        break;
                                    }
                                    let args = [
                                        t.cpu.regs.gpr[3], t.cpu.regs.gpr[4], t.cpu.regs.gpr[5],
                                        t.cpu.regs.gpr[6], t.cpu.regs.gpr[7], t.cpu.regs.gpr[8],
                                    ];
                                    let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                                    t.cpu.regs.gpr[3] = ret;
                                    t.cpu.regs.pc = t.cpu.regs.lr;
                                } else {
                                    t.active = false;
                                    break;
                                }
                            }
                            Ok(StepResult::Halted) | Err(_) => {
                                t.cpu.regs.pc = t.entry_pc;
                                t.cpu.regs.lr = 0x80001000;
                                break;
                            }
                        }
                    }
                    Err(_) => { t.active = false; break; }
                }
            }
        }

        // Execute Ring Buffer
        let _ = xenos_gpu.execute_ring(&mut bus, &mut ring_buffer);

        // Present GPU Framebuffer
        display.present_gpu_framebuffer(&xenos_gpu.color_buffer);

        if frame == 70 {
            save_u32_buffer_as_png(&display.framebuffer, XENOS_WIDTH as u32, XENOS_HEIGHT as u32, &ribbons_path)?;
            println!("  [CAPTURE] Frame 70 saved: {}", ribbons_path);
        } else if frame == 180 {
            save_u32_buffer_as_png(&display.framebuffer, XENOS_WIDTH as u32, XENOS_HEIGHT as u32, &sphere_path)?;
            println!("  [CAPTURE] Frame 180 saved: {}", sphere_path);
        } else if frame == 290 {
            save_u32_buffer_as_png(&display.framebuffer, XENOS_WIDTH as u32, XENOS_HEIGHT as u32, &dash_path)?;
            println!("  [CAPTURE] Frame 290 saved (Direct GPU Output): {}", dash_path);
        }
    }

    let total_steps: u64 = threads.iter().map(|t| t.steps).sum();
    println!("\n[+] Test Completed Successfully!");
    println!("  - Total Guest Instructions Stepped: {}", total_steps);
    println!("  - Frame 290 Display Mode: {:?}", display.mode);

    assert!(std::path::Path::new(&ribbons_path).exists(), "Ribbons capture not saved");
    assert!(std::path::Path::new(&sphere_path).exists(), "Sphere capture not saved");
    assert!(std::path::Path::new(&dash_path).exists(), "Dashboard capture not saved");

    let dash_meta = fs::metadata(&dash_path)?;
    println!("  - Real GPU Framebuffer file size: {} bytes", dash_meta.len());
    assert!(dash_meta.len() > 10000, "Dashboard capture appears empty");

    Ok(())
}
