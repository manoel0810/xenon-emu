use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::{PeImage, StfsPackage, XexFile};
use xenon_emu::memory::MemoryBus;
use std::collections::{HashMap, HashSet};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("  Xenon-Emu: Tracing Real $flash_dash.xex Guest Code     ");
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
    println!("[*] Dashboard PE Exports: {} symbols", pe.exports.len());
    for exp in &pe.exports {
        println!("  Export #0x{:04X}: {:?} @ RVA 0x{:08X}", exp.ordinal, exp.name, exp.rva);
    }

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

    let entry_vaddr = vaddr_base + pe.entry_point_rva as u64;
    println!("[*] Real Guest Entry Point: 0x{:08X}", entry_vaddr);

    let mut cpu = Cpu::new(entry_vaddr);
    cpu.regs.gpr[1] = stack_top;
    cpu.regs.gpr[3] = vaddr_base; // hModule
    cpu.regs.gpr[4] = 1;          // DLL_PROCESS_ATTACH
    cpu.regs.gpr[5] = 0;          // lpReserved
    cpu.regs.gpr[13] = kpcr_base; // KPCR
    cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top - 8, 0x80001000)?;
    bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;

    let mut syscall_counts: HashMap<String, usize> = HashMap::new();
    let mut unique_pcs: HashSet<u64> = HashSet::new();
    let mut total_instructions = 0usize;

    println!("[*] Stepping PowerPC 64 CPU core through real Dashboard DllMain instructions...");
    for step in 1..=50 {
        let pc = cpu.regs.pc;
        unique_pcs.insert(pc);

        match cpu.step(&mut bus) {
            Ok(StepResult::Ok) => {
                total_instructions += 1;
            }
            Ok(StepResult::Syscall { lev: _, pc: call_pc }) => {
                if let Some((lib, ordinal)) = thunk_map.get(&call_pc) {
                    let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                        kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                    } else {
                        kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                    };
                    let key = format!("{}!{} (#0x{:04X})", lib, fname, ordinal);
                    *syscall_counts.entry(key.clone()).or_insert(0) += 1;

                    if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                        println!("  [DASH] DllMain ExTerminateThread completed at Step {} ({} insns)!", step, total_instructions);
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
                    println!("  [!] Unmapped syscall at 0x{:08X}", call_pc);
                    break;
                }
            }
            Ok(StepResult::Halted) => break,
            Err(e) => {
                println!("  [!] Execution error at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }

    println!("\n[*] Disassembling 0x9210DF58:");
    for i in 0..10 {
        let addr = 0x9210DF58 + (i * 4) as u64;
        let insn = bus.read_u32(addr).unwrap_or(0);
        println!("  Disasm [0x{:08X}] = 0x{:08X}", addr, insn);
    }
    let stack_top2 = stack_top - 0x20000;
    cpu.regs.pc = 0x9210D608;
    cpu.regs.gpr[1] = stack_top2;
    cpu.regs.gpr[3] = 0;
    cpu.regs.gpr[4] = 1;
    cpu.regs.gpr[5] = 0;
    cpu.regs.gpr[6] = 0;
    cpu.regs.gpr[13] = kpcr_base;
    cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top2 - 8, 0x80001000)?;
    bus.write_u32(stack_top2 + 0x90 - 8, 0x80001000)?;

    for step in 1..=300 {
        let pc = cpu.regs.pc;
        unique_pcs.insert(pc);
        let raw_insn = bus.read_u32(pc).unwrap_or(0);

        match cpu.step(&mut bus) {
            Ok(StepResult::Ok) => {
                total_instructions += 1;
                if step <= 20 || step % 50 == 0 {
                    println!("  [PHASE 2] Step {:02} | PC: 0x{:08X} | Insn: 0x{:08X} | SP: 0x{:08X}",
                        step, pc, raw_insn, cpu.regs.gpr[1]
                    );
                }
            }
            Ok(StepResult::Syscall { lev: _, pc: call_pc }) => {
                if let Some((lib, ordinal)) = thunk_map.get(&call_pc) {
                    let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                        kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                    } else {
                        kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                    };
                    let key = format!("{}!{} (#0x{:04X})", lib, fname, ordinal);
                    *syscall_counts.entry(key.clone()).or_insert(0) += 1;

                    let args = [
                        cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5],
                        cpu.regs.gpr[6], cpu.regs.gpr[7], cpu.regs.gpr[8],
                    ];
                    let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                    println!("  [PHASE 2 SYSCALL] {}!{} (#0x{:04X}) -> ret 0x{:X}", lib, fname, ordinal, ret);

                    if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                        println!("  [DASH] Master Initializer complete! Step: {}, Total Insns: {}", step, total_instructions);
                        break;
                    }

                    cpu.regs.gpr[3] = ret;
                    cpu.regs.pc = cpu.regs.lr;
                } else {
                    println!("  [!] Unmapped syscall at 0x{:08X}", call_pc);
                    break;
                }
            }
            Ok(StepResult::Halted) => {
                println!("  [!] CPU halted at 0x{:08X}", pc);
                break;
            }
            Err(e) => {
                println!("  [!] Execution error at 0x{:08X}: {}", pc, e);
                break;
            }
        }
    }

    println!("\n[+] Total real instructions executed: {}", total_instructions);
    println!("[+] Unique program counter addresses visited: {}", unique_pcs.len());
    println!("\n[+] Syscalls intercepted and satisfied by HLE Kernel:");
    let mut sorted_syscalls: Vec<_> = syscall_counts.iter().collect();
    sorted_syscalls.sort_by_key(|(_, count)| **count);
    sorted_syscalls.reverse();
    for (name, count) in sorted_syscalls {
        println!("  - {:<45} called {} times", name, count);
    }

    Ok(())
}
