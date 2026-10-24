//! Pre-Metro dashboard path (blades dash / `$flash_dash.xex`): scripted init phases.
//! Not used by the 17559 Metro dashboard; kept unchanged until it is removed or rewritten.

use std::collections::HashMap;
use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::kernel::KernelDispatcher;
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
