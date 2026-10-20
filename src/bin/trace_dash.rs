use byteorder::{BigEndian, ByteOrder};
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

    // Trampolines
    bus.write_u32(0x80000000, 0x44000042)?; // sc 1
    bus.write_u32(0x80000004, 0x4E800020)?; // blr
    thunk_map.insert(0x80000000, ("xboxkrnl.exe".to_string(), 41)); // ExTerminateThread
    thunk_map.insert(0x00000000, ("xboxkrnl.exe".to_string(), 41));

    let kernel = KernelDispatcher::new();
    println!("[*] Thunk 0x9268E9C4: {:?}", thunk_map.get(&0x9268E9C4));
    println!("\n[*] Listing Thunks from 0x9268EA00 to 0x9268EC00:");
    let mut sorted_thunks: Vec<_> = thunk_map.iter().collect();
    sorted_thunks.sort_by_key(|k| k.0);
    for (addr, (lib, ord)) in sorted_thunks {
        if *addr >= 0x9268EA00 && *addr <= 0x9268ECC0 {
            let func_name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                kernel.resolve_xboxkrnl(*ord).map(|f| f.name).unwrap_or("unknown")
            } else {
                kernel.resolve_xam(*ord).map(|f| f.name).unwrap_or("unknown")
            };
            println!("  0x{:08X} -> {}!#{:<4} ({})", addr, lib, ord, func_name);
        }
    }

    println!("\n[*] All Import Libraries and Ordinals:");
    for lib in &dash_xex.import_libraries {
        let mut ords = Vec::new();
        for &rec_addr in &lib.import_table {
            if let Some((_, ord)) = thunk_map.get(&(rec_addr as u64)) {
                let name = if lib.name.eq_ignore_ascii_case("xboxkrnl.exe") {
                    kernel.resolve_xboxkrnl(*ord).map(|f| f.name).unwrap_or("UNKNOWN")
                } else {
                    kernel.resolve_xam(*ord).map(|f| f.name).unwrap_or("UNKNOWN")
                };
                ords.push((*ord, name));
            }
        }
        ords.sort_by_key(|k| k.0);
        println!("  - Library '{}' ({} functions):", lib.name, ords.len());
        for (ord, name) in ords {
            println!("      #{:<4} -> {}", ord, name);
        }
    }

    let entry_vaddr = dash_xex.entry_point as u64;
    let target_vaddr = 0x921088A0u64;
    println!("\n[*] Disassembly of 0x921088A0 to 0x92108920 (Main Dash Caller):");
    for off in (0..0x80).step_by(4) {
        let addr = target_vaddr + off;
        let insn = bus.read_u32(addr)?;
        let op = (insn >> 26) & 0x3F;
        println!("    0x{:08X}: {:08X} (op {:02})", addr, insn, op);
    }

    let target_func = 0x9210D608i64;
    println!("\n[*] Scanning .text section for callers of 0x{:08X}:", target_func);
    let text_start = 0x92105000u64;
    let text_end = text_start + 0x0058A894;

    for addr in (text_start..text_end).step_by(4) {
        if let Ok(insn) = bus.read_u32(addr) {
            let op = (insn >> 26) & 0x3F;
            if op == 18 {
                let lk = insn & 0x1;
                if lk == 1 {
                    let li = (insn & 0x03FF_FFFC) as i32;
                    let li = ((li << 6) >> 6) as i64;
                    let aa = (insn >> 1) & 0x1;
                    let target = if aa != 0 { li } else { addr as i64 + li };
                    if target == target_func {
                        println!("  [CALLER OF 0x9210D608] at 0x{:08X} (bl 0x{:08X})", addr, target);
                    }
                }
            }
        }
    }

    let mut cpu = Cpu::new(0x9210DFD0);
    cpu.regs.gpr[1] = vaddr_base + dash_xex.image_size as u64 - 0x10000;
    cpu.regs.lr = 0x80000000;

    let mut total_steps = 0;
    for step in 1..=1000 {
        let pc = cpu.regs.pc;
        match bus.read_u32(pc) {
            Ok(insn) => {
                match cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        total_steps += 1;
                        if step <= 40 || step % 50 == 0 {
                            println!("  [{:04}] PC: 0x{:08X} | Opcode: 0x{:08X} | SP: 0x{:08X}", step, pc, insn, cpu.regs.gpr[1]);
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc: call_pc }) => {
                        if let Some((lib, ord)) = thunk_map.get(&call_pc) {
                            println!("  [SYSCALL #{:04}] lev={} -> {}!#{}", step, lev, lib, ord);
                            if *ord == 41 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!("  -> ExTerminateThread called! DllMain routine completed successfully!");
                                break;
                            }
                            let args = [
                                cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5],
                                cpu.regs.gpr[6], cpu.regs.gpr[7], cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ord, &args);
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
    println!("[+] Trace completed. Total instructions executed: {}", total_steps);

    Ok(())
}
