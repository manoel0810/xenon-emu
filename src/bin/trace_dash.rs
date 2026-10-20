use std::fs;
use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::{PeImage, StfsPackage, XexFile};
use xenon_emu::memory::MemoryBus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Trace & Reverse Engineer $flash_dash.xex Execution  ");
    println!("========================================================");

    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    println!("[+] STFS Package contains {} files:", stfs.entries.len());
    for e in &stfs.entries {
        println!("    - {:<30} ({} bytes)", e.name, e.file_size);
    }
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    println!("[*] Dashboard XEX Header Summary:");
    println!("    - Load Address:  0x{:08X}", dash_xex.load_address);
    println!("    - Image Size:    0x{:08X} ({} MB)", dash_xex.image_size, dash_xex.image_size / (1024 * 1024));
    println!("    - XEX Entry:     0x{:08X}", dash_xex.entry_point);

    if let Some(ref exec_info) = dash_xex.execution_info {
        println!("    - Media ID:      0x{:08X}", exec_info.media_id);
        println!("    - Title ID:      0x{:08X}", exec_info.title_id);
        println!("    - Platform:      0x{:02X}", exec_info.platform);
        println!("    - Exec Table:    0x{:08X}", exec_info.executable_table);
        println!("    - Disc Number:   {}/{}", exec_info.disc_number, exec_info.disc_count);
    }

    println!("\n[*] Optional Headers Map:");
    for (&key, &val) in &dash_xex.optional_headers {
        println!("    - Key 0x{:08X} => Value/Offset 0x{:08X}", key, val);
    }

    let decrypted_payload = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let uncompressed_pe = dash_xex.decompress_image(&decrypted_payload)?;
    let pe_image = PeImage::parse(&uncompressed_pe)?;

    let vaddr_base = dash_xex.load_address as u64;
    println!("\n[*] PE Image:");
    println!("    - PE Entry RVA:  0x{:08X} (VAddr 0x{:016X})", pe_image.entry_point_rva, vaddr_base + pe_image.entry_point_rva as u64);
    println!("    - Sections:      {}", pe_image.sections.len());
    println!("    - Exports:       {}", pe_image.exports.len());

    for exp in &pe_image.exports {
        println!("      [EXPORT] Ord #{:<3} RVA: 0x{:08X} Name: {:?}", exp.ordinal, exp.rva, exp.name);
    }

    let mut bus = MemoryBus::new();
    pe_image.load_into_memory(&uncompressed_pe, vaddr_base, &mut bus)?;
    let mut thunk_map = dash_xex.setup_import_thunks(&mut bus)?;

    // Trampolines (place at 0x80001000 to avoid physical address 0)
    bus.write_u32(0x80001000, 0x44000042)?; // sc 1
    bus.write_u32(0x80001004, 0x4E800020)?; // blr
    thunk_map.insert(0x80001000, ("xboxkrnl.exe".to_string(), 0x0019)); // ExTerminateThread (0x19 = 25)

    let mut kernel = KernelDispatcher::new();
    println!("[*] Listing sample thunks:");
    let mut sorted_thunks: Vec<_> = thunk_map.clone().into_iter().collect();
    sorted_thunks.sort_by_key(|k| k.0);
    for (addr, (lib, ord)) in &sorted_thunks {
        if *addr >= 0x9268EA00 && *addr <= 0x9268EB00 {
            let func_name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                kernel.resolve_xboxkrnl(*ord).unwrap_or("unknown")
            } else {
                kernel.resolve_xam(*ord).unwrap_or("unknown")
            };
            println!("  0x{:08X} -> {}!#{:<4} (0x{:04X}) ({})", addr, lib, ord, ord, func_name);
        }
    }

    let entry_vaddr = dash_xex.entry_point as u64;
    println!("\n[*] Executing $flash_dash.xex from Entry Point 0x{:08X}...", entry_vaddr);

    let stack_top = vaddr_base + dash_xex.image_size as u64 - 0x10000;

    // Setup KPCR at 0x7FE00000
    let kpcr_base = 0x7FE00000u64;
    let tls_base = 0x7FE01000u64;
    bus.write_u32(kpcr_base + 0x00, tls_base as u32)?; // tls_ptr
    bus.write_u32(kpcr_base + 0x30, kpcr_base as u32)?; // pcr_ptr
    bus.write_u32(kpcr_base + 0x70, stack_top as u32)?; // stack_base_ptr
    bus.write_u32(kpcr_base + 0x74, (stack_top - 0x40000) as u32)?; // stack_end_ptr
    bus.write_u32(kpcr_base + 0x100, (kpcr_base + 0x200) as u32)?; // current_thread
    bus.write_u8(kpcr_base + 0x10C, 0)?; // current_cpu = 0
    bus.write_u32(kpcr_base + 0x14C, 1)?; // thread_id = 1
    bus.write_u32(kpcr_base + 0x150, 0)?; // dpc_active = 0
    bus.write_u32(kpcr_base + 0x160, 0)?; // last_error = 0

    let mut cpu = Cpu::new(entry_vaddr);
    cpu.regs.gpr[1] = stack_top;
    cpu.regs.gpr[3] = vaddr_base; // hModule (0x92000000)
    cpu.regs.gpr[4] = 1;          // DLL_PROCESS_ATTACH (1)
    cpu.regs.gpr[5] = 0;          // lpReserved (0)
    cpu.regs.gpr[13] = kpcr_base; // r13: KPCR
    cpu.regs.lr = 0x80001000;

    // In PowerPC 64 ELF/PE ABI, caller's LR is saved in stack frame
    bus.write_u32(stack_top - 8, 0x80001000)?;
    bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;

    let mut total_steps = 0;
    for step in 1..=2000 {
        let pc = cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        total_steps += 1;
                        if step <= 60 || step % 50 == 0 {
                            println!("  [{:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! Execution sequence completed successfully!");
                                break;
                            }
                            let args = [
                                cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5],
                                cpu.regs.gpr[6], cpu.regs.gpr[7], cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            cpu.regs.gpr[3] = ret;
                            cpu.regs.pc = cpu.regs.lr;
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 1 (DllMain) completed in {} steps with status r3={}", total_steps, cpu.regs.gpr[3]);

    // ==========================================================
    // PHASE 2: Execute Master Dashboard Initializer (0x9210D608)
    // ==========================================================
    println!("\n[*] Executing Dashboard Master Initializer (0x9210D608)...");
    let mut dash_main_cpu = Cpu::new(0x9210D608);
    let stack_top2 = stack_top - 0x20000;
    dash_main_cpu.regs.gpr[1] = stack_top2;
    dash_main_cpu.regs.gpr[3] = 0; // r3: 0
    dash_main_cpu.regs.gpr[4] = 1; // r4: 1
    dash_main_cpu.regs.gpr[5] = 0; // r5: 0
    dash_main_cpu.regs.gpr[6] = 0; // r6: 0
    dash_main_cpu.regs.gpr[13] = kpcr_base;
    dash_main_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top2 - 8, 0x80001000)?;
    bus.write_u32(stack_top2 + 0x90 - 8, 0x80001000)?;

    let mut phase2_steps = 0;
    for step in 1..=5000 {
        let pc = dash_main_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match dash_main_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase2_steps += 1;
                        if step <= 60 || step % 100 == 0 {
                            println!("  [DASH INIT {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, dash_main_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH INIT SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! Dashboard Initialization routine completed!");
                                break;
                            }
                            let args = [
                                dash_main_cpu.regs.gpr[3], dash_main_cpu.regs.gpr[4], dash_main_cpu.regs.gpr[5],
                                dash_main_cpu.regs.gpr[6], dash_main_cpu.regs.gpr[7], dash_main_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            dash_main_cpu.regs.gpr[3] = ret;
                            dash_main_cpu.regs.pc = dash_main_cpu.regs.lr;
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 2 completed. Total instructions executed: {}", phase2_steps);

    // ==========================================================
    // PHASE 3: Execute Main Dashboard Loop Caller (0x921088A0)
    // ==========================================================
    println!("\n[*] Executing Dashboard Main Sequence (0x921088A0)...");
    let mut dash_loop_cpu = Cpu::new(0x921088A0);
    let stack_top3 = stack_top - 0x40000;
    dash_loop_cpu.regs.gpr[1] = stack_top3;
    dash_loop_cpu.regs.gpr[3] = 0;
    dash_loop_cpu.regs.gpr[4] = 1;
    dash_loop_cpu.regs.gpr[13] = kpcr_base;
    dash_loop_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top3 - 8, 0x80001000)?;
    bus.write_u32(stack_top3 + 0x80 - 8, 0x80001000)?;

    let mut phase3_steps = 0;
    for step in 1..=5000 {
        let pc = dash_loop_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match dash_loop_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase3_steps += 1;
                        if step <= 60 || step % 100 == 0 {
                            println!("  [DASH MAIN {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, dash_loop_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH MAIN SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! Dashboard main sequence completed!");
                                break;
                            }
                            let args = [
                                dash_loop_cpu.regs.gpr[3], dash_loop_cpu.regs.gpr[4], dash_loop_cpu.regs.gpr[5],
                                dash_loop_cpu.regs.gpr[6], dash_loop_cpu.regs.gpr[7], dash_loop_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            dash_loop_cpu.regs.gpr[3] = ret;
                            dash_loop_cpu.regs.pc = dash_loop_cpu.regs.lr;
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 3 completed. Total instructions executed: {}", phase3_steps);

    // ==========================================================
    // PHASE 4: Execute Genuine Dashboard Graphics & Engine Init (0x9212A34C)
    // ==========================================================
    println!("\n[*] Executing Genuine Dashboard Graphics Init (0x9212A34C)...");
    let mut dash_gfx_cpu = Cpu::new(0x9212A34C);
    let stack_top4 = stack_top - 0x60000;
    dash_gfx_cpu.regs.gpr[1] = stack_top4;
    dash_gfx_cpu.regs.gpr[3] = vaddr_base;
    dash_gfx_cpu.regs.gpr[4] = 0;
    dash_gfx_cpu.regs.gpr[13] = kpcr_base;
    dash_gfx_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top4 - 8, 0x80001000)?;
    bus.write_u32(stack_top4 + 0xA0 - 8, 0x80001000)?;

    let mut phase4_steps = 0;
    for step in 1..=1000 {
        let pc = dash_gfx_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match dash_gfx_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase4_steps += 1;
                        if step <= 100 || step % 100 == 0 {
                            println!("  [DASH GFX {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, dash_gfx_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH GFX SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! Graphics initialization sequence completed!");
                                break;
                            }
                            let args = [
                                dash_gfx_cpu.regs.gpr[3], dash_gfx_cpu.regs.gpr[4], dash_gfx_cpu.regs.gpr[5],
                                dash_gfx_cpu.regs.gpr[6], dash_gfx_cpu.regs.gpr[7], dash_gfx_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            dash_gfx_cpu.regs.gpr[3] = ret;
                            dash_gfx_cpu.regs.pc = dash_gfx_cpu.regs.lr;
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 4 completed. Total instructions executed: {}", phase4_steps);

    // ==========================================================
    // PHASE 5: Execute Genuine VdInitializeEngines Master Sequence (0x9210EA48)
    // ==========================================================
    println!("\n[*] Executing Genuine Dashboard VdInitializeEngines Master Sequence (0x9210EA48)...");
    let mut dash_eng_cpu = Cpu::new(0x9210EA48);
    let stack_top5 = stack_top - 0x80000;
    // Initialize empty circular LIST_ENTRY at 0x926E95C4 (Flink=self, Blink=self)
    bus.write_u32(0x926E95C4, 0x926E95C4)?;
    bus.write_u32(0x926E95C8, 0x926E95C4)?;

    dash_eng_cpu.regs.gpr[1] = stack_top5;
    dash_eng_cpu.regs.gpr[3] = 0;
    dash_eng_cpu.regs.gpr[4] = 0;
    dash_eng_cpu.regs.gpr[13] = kpcr_base;
    dash_eng_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top5 - 8, 0x80001000)?;
    bus.write_u32(stack_top5 + 0x80 - 8, 0x80001000)?;

    let mut phase5_steps = 0;
    for step in 1..=1000 {
        let pc = dash_eng_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match dash_eng_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase5_steps += 1;
                        if pc == 0x9210EA88 {
                            println!("  [DEBUG 0x9210EA88] r30 = 0x{:016X}, r31 = 0x{:016X}", dash_eng_cpu.regs.gpr[30], dash_eng_cpu.regs.gpr[31]);
                        }
                        if step <= 100 || step % 100 == 0 {
                            println!("  [DASH ENG {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, dash_eng_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH ENG SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! VdInitializeEngines sequence completed!");
                                break;
                            }
                            let args = [
                                dash_eng_cpu.regs.gpr[3], dash_eng_cpu.regs.gpr[4], dash_eng_cpu.regs.gpr[5],
                                dash_eng_cpu.regs.gpr[6], dash_eng_cpu.regs.gpr[7], dash_eng_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            dash_eng_cpu.regs.gpr[3] = ret;
                            dash_eng_cpu.regs.pc = dash_eng_cpu.regs.lr;
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 5 completed. Total instructions executed: {}", phase5_steps);

    // ==========================================================
    // PHASE 6: Execute Genuine Dashboard Worker & Content Thread (0x92107280)
    // ==========================================================
    println!("\n[*] Executing Genuine Dashboard Worker & Content Thread (0x92107280)...");
    let mut dash_bg_cpu = Cpu::new(0x92107280);
    let stack_top6 = stack_top - 0xA0000;
    dash_bg_cpu.regs.gpr[1] = stack_top6;
    dash_bg_cpu.regs.gpr[3] = 0;
    dash_bg_cpu.regs.gpr[4] = 0;
    dash_bg_cpu.regs.gpr[13] = kpcr_base;
    dash_bg_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top6 - 8, 0x80001000)?;
    bus.write_u32(stack_top6 + 0x320 - 8, 0x80001000)?;

    let mut phase6_steps = 0;
    for step in 1..=2000 {
        let pc = dash_bg_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match dash_bg_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase6_steps += 1;
                        if step <= 60 || step % 50 == 0 {
                            println!("  [DASH BG {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, dash_bg_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH BG SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            let args = [
                                dash_bg_cpu.regs.gpr[3], dash_bg_cpu.regs.gpr[4], dash_bg_cpu.regs.gpr[5],
                                dash_bg_cpu.regs.gpr[6], dash_bg_cpu.regs.gpr[7], dash_bg_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            dash_bg_cpu.regs.gpr[3] = ret;
                            dash_bg_cpu.regs.pc = dash_bg_cpu.regs.lr;

                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! Worker sequence completed!");
                                break;
                            }
                            if *ord == 0x0101 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> NtYieldExecution called! Worker thread yielded and resumed at PC 0x{:08X}", dash_bg_cpu.regs.pc);
                            }
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 6 completed. Total instructions executed: {}", phase6_steps);

    // ==========================================================
    // PHASE 7: Execute Authentic Dashboard Controller Thread (0x921C9830)
    // ==========================================================
    println!("\n[*] Executing Authentic Dashboard Controller Thread from Live Console (0x921C9830)...");
    let mut ctrl_cpu = Cpu::new(0x921C9830);
    let stack_top7 = stack_top - 0xC0000;
    ctrl_cpu.regs.gpr[1] = stack_top7;
    ctrl_cpu.regs.gpr[3] = 0;
    ctrl_cpu.regs.gpr[4] = 0;
    ctrl_cpu.regs.gpr[13] = kpcr_base;
    ctrl_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top7 - 8, 0x80001000)?;
    bus.write_u32(stack_top7 + 0x100 - 8, 0x80001000)?;

    let mut phase7_steps = 0;
    for step in 1..=1000 {
        let pc = ctrl_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match ctrl_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase7_steps += 1;
                        if step <= 40 || step % 50 == 0 {
                            println!("  [DASH CTRL {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, ctrl_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH CTRL SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            let args = [
                                ctrl_cpu.regs.gpr[3], ctrl_cpu.regs.gpr[4], ctrl_cpu.regs.gpr[5],
                                ctrl_cpu.regs.gpr[6], ctrl_cpu.regs.gpr[7], ctrl_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            ctrl_cpu.regs.gpr[3] = ret;
                            ctrl_cpu.regs.pc = ctrl_cpu.regs.lr;

                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! Controller sequence completed!");
                                break;
                            }
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 7 completed. Total instructions executed: {}", phase7_steps);

    // ==========================================================
    // PHASE 8: Execute Authentic XUI Scene Graph Thread (0x9281FBC0)
    // ==========================================================
    println!("\n[*] Executing Authentic XUI Scene Graph Thread from Live Console (0x9281FBC0)...");
    let mut xui_cpu = Cpu::new(0x9281FBC0);
    let stack_top8 = stack_top - 0xE0000;
    xui_cpu.regs.gpr[1] = stack_top8;
    xui_cpu.regs.gpr[3] = 0;
    xui_cpu.regs.gpr[4] = 0;
    xui_cpu.regs.gpr[13] = kpcr_base;
    xui_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top8 - 8, 0x80001000)?;
    bus.write_u32(stack_top8 + 0x100 - 8, 0x80001000)?;

    let mut phase8_steps = 0;
    for step in 1..=1000 {
        let pc = xui_cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match xui_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase8_steps += 1;
                        if step <= 40 || step % 50 == 0 {
                            println!("  [DASH XUI {:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, xui_cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ord).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ord).unwrap_or("Unknown")
                            };
                            println!("  [DASH XUI SYSCALL #{:04}] lev={} -> {}!{} (#0x{:04X} / {})", step, lev, lib, name, ord, ord);
                            let args = [
                                xui_cpu.regs.gpr[3], xui_cpu.regs.gpr[4], xui_cpu.regs.gpr[5],
                                xui_cpu.regs.gpr[6], xui_cpu.regs.gpr[7], xui_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args, &mut bus);
                            xui_cpu.regs.gpr[3] = ret;
                            xui_cpu.regs.pc = xui_cpu.regs.lr;

                            if *ord == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! XUI sequence completed!");
                                break;
                            }
                        } else {
                            println!("  [UNMAPPED SYSCALL] PC: 0x{:016X}", call_pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => {
                        println!("  [HALTED] at PC: 0x{:016X}", pc);
                        break;
                    }
                    Err(e) => {
                        println!("  [TRAP] PC: 0x{:08X} Opcode: 0x{:08X}: {}", pc, insn, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [BUS ERROR] at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 8 completed. Total instructions executed: {}", phase8_steps);

    Ok(())
}
