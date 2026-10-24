//! Pre-Metro dashboard path (blades dash / `$flash_dash.xex`): scripted init phases and a synthetic PM4 stream.
//! Not used by the 17559 Metro dashboard; kept unchanged until it is removed or rewritten.

use std::collections::HashMap;
use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::gpu::{
    build_draw_indexed, build_load_alu_constant, build_set_context_reg, build_wait_for_idle,
    RingBuffer, XenosGpu, PA_SC_WINDOW_OFFSET, RB_COLOR_INFO, VGT_DMA_BASE, VGT_DMA_SIZE,
    VGT_DMA_STRIDE,
};
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::{XurScene, XusTable};
use xenon_emu::memory::MemoryBus;

/// Phases 1-5: DllMain, master initializer, main sequence, graphics init and VdInitializeEngines.
pub fn run_init_phases(
    bus: &mut MemoryBus,
    kernel: &mut KernelDispatcher,
    thunk_map: &HashMap<u64, (String, u16)>,
    real_entry_vaddr: u64,
    vaddr_base: u64,
    stack_top: u64,
    kpcr_base: u64,
) -> Result<(), Box<dyn std::error::Error>> {
        // ==========================================================
        // PHASE 1: DllMain (Process Attach) @ 0x9210D308
        // ==========================================================
        println!("\n[*] [PHASE 1] Executing Dashboard DllMain (Process Attach @ 0x{:08X})...", real_entry_vaddr);
        let mut dash_cpu = Cpu::new(real_entry_vaddr);
    dash_cpu.regs.gpr[1] = stack_top;
    dash_cpu.regs.gpr[3] = vaddr_base; // r3: hModule (0x92000000)
    dash_cpu.regs.gpr[4] = 1;          // r4: DLL_PROCESS_ATTACH (1)
    dash_cpu.regs.gpr[5] = 0;          // r5: lpReserved (0)
    dash_cpu.regs.gpr[13] = kpcr_base; // r13: KPCR
    dash_cpu.regs.lr = 0x80001000;     // LR: ExTerminateThread stub
    bus.write_u32(stack_top - 8, 0x80001000)?;
    bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;

    let mut phase1_steps = 0;
    for step in 1..=200 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(bus) {
                    Ok(StepResult::Ok) => {
                        phase1_steps += 1;
                        if step <= 20 || step % 50 == 0 {
                            println!(
                                "  [DASH EXEC] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH HLE SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: DllMain completed with status 1!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH HLE SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH SYSCALL] Unmapped syscall at PC: 0x{:016X} (lev={})", pc, lev);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH EXEC] Execution trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH EXEC] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 1 (DllMain) completed! ({} instructions stepped)", phase1_steps);

    // ==========================================================
    // PHASE 2: Dashboard Master Initializer @ 0x9210D608
    // ==========================================================
    println!("\n[*] [PHASE 2] Executing Dashboard Master Initializer (0x9210D608)...");
    let stack_top2 = stack_top - 0x20000;
    dash_cpu.regs.pc = 0x9210D608;
    dash_cpu.regs.gpr[1] = stack_top2;
    dash_cpu.regs.gpr[3] = 0;
    dash_cpu.regs.gpr[4] = 1;
    dash_cpu.regs.gpr[5] = 0;
    dash_cpu.regs.gpr[6] = 0;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top2 - 8, 0x80001000)?;
    bus.write_u32(stack_top2 + 0x90 - 8, 0x80001000)?;

    let mut phase2_steps = 0;
    for step in 1..=500 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(bus) {
                    Ok(StepResult::Ok) => {
                        phase2_steps += 1;
                        if step <= 20 || step % 50 == 0 {
                            println!(
                                "  [DASH INIT] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH INIT SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: Master Initializer complete!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH INIT SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH INIT SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH INIT] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH INIT] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 2 (Master Initializer) completed! ({} instructions stepped)", phase2_steps);

    // ==========================================================
    // PHASE 3: Dashboard Main Loop & UI Sequence @ 0x921088A0
    // ==========================================================
    println!("\n[*] [PHASE 3] Executing Dashboard Main Sequence (0x921088A0)...");
    let stack_top3 = stack_top - 0x40000;
    dash_cpu.regs.pc = 0x921088A0;
    dash_cpu.regs.gpr[1] = stack_top3;
    dash_cpu.regs.gpr[3] = 0;
    dash_cpu.regs.gpr[4] = 1;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top3 - 8, 0x80001000)?;
    bus.write_u32(stack_top3 + 0x80 - 8, 0x80001000)?;

    let mut phase3_steps = 0;
    for step in 1..=1000 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(bus) {
                    Ok(StepResult::Ok) => {
                        phase3_steps += 1;
                        if step <= 20 || step % 50 == 0 {
                            println!(
                                "  [DASH MAIN] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH MAIN SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: Dashboard main sequence completed!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH MAIN SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH MAIN SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH MAIN] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH MAIN] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 3 (Dashboard Main Sequence) completed! ({} instructions stepped)", phase3_steps);

    // ==========================================================
    // PHASE 4: Genuine Dashboard Graphics Init @ 0x9212A34C
    // ==========================================================
    println!("\n[*] [PHASE 4] Executing Genuine Dashboard Graphics Init (0x9212A34C)...");
    let stack_top4 = stack_top - 0x60000;
    dash_cpu.regs.pc = 0x9212A34C;
    dash_cpu.regs.gpr[1] = stack_top4;
    dash_cpu.regs.gpr[3] = vaddr_base;
    dash_cpu.regs.gpr[4] = 0;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top4 - 8, 0x80001000)?;
    bus.write_u32(stack_top4 + 0xA0 - 8, 0x80001000)?;

    let mut phase4_steps = 0;
    for step in 1..=500 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(bus) {
                    Ok(StepResult::Ok) => {
                        phase4_steps += 1;
                        if step <= 15 || step % 50 == 0 {
                            println!(
                                "  [DASH GFX] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH GFX SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: Graphics initialization sequence completed!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH GFX SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH GFX SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH GFX] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH GFX] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 4 (Graphics Subsystem Init) completed! ({} instructions stepped)", phase4_steps);

    // ==========================================================
    // PHASE 5: Genuine Dashboard VdInitializeEngines Master Sequence @ 0x9210EA48
    // ==========================================================
    println!("\n[*] [PHASE 5] Executing Genuine Dashboard VdInitializeEngines Master Sequence (0x9210EA48)...");
    let stack_top5 = stack_top - 0x80000;
    // Initialize circular LIST_ENTRY at 0x926E95C4
    bus.write_u32(0x926E95C4, 0x926E95C4)?;
    bus.write_u32(0x926E95C8, 0x926E95C4)?;

    dash_cpu.regs.pc = 0x9210EA48;
    dash_cpu.regs.gpr[1] = stack_top5;
    dash_cpu.regs.gpr[3] = 0;
    dash_cpu.regs.gpr[4] = 0;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top5 - 8, 0x80001000)?;
    bus.write_u32(stack_top5 + 0x80 - 8, 0x80001000)?;

    let mut phase5_steps = 0;
    for step in 1..=500 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(bus) {
                    Ok(StepResult::Ok) => {
                        phase5_steps += 1;
                        if step <= 15 || step % 50 == 0 {
                            println!(
                                "  [DASH ENG] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH ENG SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: VdInitializeEngines sequence complete!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH ENG SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH ENG SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH ENG] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH ENG] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
        println!("[+] Phase 5 (VdInitializeEngines Master Sequence) completed! ({} instructions stepped)", phase5_steps);
    Ok(())
}

/// CPU-emitted PM4 stream (old invented opcodes) plus the XUI barycentric rasterizer demo.
pub fn emit_synthetic_stream(
    bus: &mut MemoryBus,
    kernel: &mut KernelDispatcher,
    ring_buffer: &mut RingBuffer,
    xenos_gpu: &mut XenosGpu,
    ring_base: u64,
    ring_size: usize,
    loaded_scenes: &[XurScene],
    xus_table: &Option<XusTable>,
) -> Result<(), Box<dyn std::error::Error>> {
        // Link Primary Ring Buffer with Xboxkrnl Video Subsystem
        let rb_ret = kernel.dispatch("xboxkrnl.exe", 0x01C3, &[ring_base, ring_size as u64], bus);
        println!("  -> VdInitializeRingBuffer(0x{:08X}, {} KB): status={}", ring_base, ring_size / 1024, rb_ret);

        // CPU emits Direct3D / XUI PM4 packet stream to the Ring Buffer:
        println!("[*] CPU emitting Direct3D / XUI PM4 packet stream into Ring Buffer:");
        // 1. Set Context Register: RB_COLOR_INFO (RenderTarget: 1280x720 32-bit ARGB at 0x70000000)
        let pkt_color = build_set_context_reg(RB_COLOR_INFO, 0x7000_0000);
        ring_buffer.write_packet(bus, &pkt_color)?;
        println!("  [PM4 EMIT] SET_CONTEXT_REG (RB_COLOR_INFO: 0x70000000) [{} dwords]", pkt_color.len());

        // 2. Set Context Register: PA_SC_WINDOW_OFFSET (Viewport: (0, 0))
        let pkt_vport = build_set_context_reg(PA_SC_WINDOW_OFFSET, 0x0000_0000);
        ring_buffer.write_packet(bus, &pkt_vport)?;
        println!("  [PM4 EMIT] SET_CONTEXT_REG (PA_SC_WINDOW_OFFSET: 0x0) [{} dwords]", pkt_vport.len());

        // 3. Load ALU Constants (c0..c3: 4x4 Matrix for UI Projection, c4: Diffuse Tint)
        let shader_constants = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ];
        let pkt_constants = build_load_alu_constant(0, &shader_constants);
        ring_buffer.write_packet(bus, &pkt_constants)?;
        println!("  [PM4 EMIT] LOAD_ALU_CONSTANT (c0..c4: Matrix + Diffuse Tint) [{} dwords]", pkt_constants.len());

        // 4. Stream Vertex Buffer in guest RAM at 0x10060000 (VGT DMA)
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
        ring_buffer.write_packet(bus, &pkt_dma_base)?;
        ring_buffer.write_packet(bus, &pkt_dma_size)?;
        ring_buffer.write_packet(bus, &pkt_dma_stride)?;
        println!("  [PM4 EMIT] SET_CONTEXT_REG (VGT_DMA_BASE: 0x{:08X}, Size: 4, Stride: 24)", vgt_addr);

        // 5. Index buffer setup in guest RAM at 0x10050000
        let index_ptr: u32 = 0x1005_0000;
        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0]; // 2 Triangles for UI Quad
        for (i, &idx) in indices.iter().enumerate() {
            bus.write_u16(index_ptr as u64 + (i * 2) as u64, idx)?;
        }

        // 6. Emit Draw Call: PM4_DRAW_INDX (Primitive: Triangles, 6 indices)
        let pkt_draw = build_draw_indexed(0x04, 6, index_ptr);
        ring_buffer.write_packet(bus, &pkt_draw)?;
        println!("  [PM4 EMIT] DRAW_INDX (Primitive: Triangles, 6 Indices, Base: 0x{:08X}) [{} dwords]", index_ptr, pkt_draw.len());

        // 7. Emit WAIT_FOR_IDLE fence
        let pkt_idle = build_wait_for_idle();
        ring_buffer.write_packet(bus, &pkt_idle)?;
        println!("  [PM4 EMIT] WAIT_FOR_IDLE (Pipeline Flush & Fence) [{} dwords]", pkt_idle.len());

        println!("\n[*] Executing Xenos GPU Command Processor (CP) loop on Primary Ring Buffer...");
        let executed_count = xenos_gpu.execute_ring(bus, ring_buffer)?;
        println!("[+] Xenos GPU CP successfully processed {} PM4 packets from Ring Buffer!", executed_count);
        println!("    - Total Packets Processed: {}", xenos_gpu.packets_executed);
        println!("    - Draw Calls Executed:     {}", xenos_gpu.draw_calls);
        println!("    - Primitives Drawn:        {}", xenos_gpu.primitives_drawn);

        // ==========================================================
        // ETAPA 3: Direct3D 9 Subsystem & Barycentric Triangle Rasterizer
        // ==========================================================
        println!("\n>>> [ETAPA 3] Direct3D 9 Subsystem & XUI Barycentric Rasterizer <<<");
        if let Some(games_scene) = loaded_scenes.iter().find(|s| s.name.contains("games")) {
            println!("[*] Rasterizing authentic XUI Scene '{}' with D3D9 barycentric pipeline...", games_scene.name);
            xenos_gpu.render_xui_scene(games_scene, xus_table.as_ref(), 0x0010_7C10, 0x000B_4D0B);
            println!("[+] Direct3D 9 Subsystem successfully rasterized UI nodes into on-chip color buffer!");
            println!("    - Triangles Shaded:        {}", xenos_gpu.primitives_drawn);
            println!("    - Active Draw Calls:       {}", xenos_gpu.draw_calls);
            println!("    - Color Buffer Resolution: 1280x720 32-bit ARGB (EDRAM Target)");
        }
    Ok(())
}
