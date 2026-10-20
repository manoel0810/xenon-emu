use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::{PeImage, XexFile};
use xenon_emu::memory::MemoryBus;
use std::collections::{HashMap, HashSet};
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("  Xenon-Emu: Tracing Dashboard Metro 17559 Execution    ");
    println!("========================================================");

    let dash_path = "../SystemUpdate_17559_USB/$SystemUpdate/dash.xex";
    let dash_bytes = fs::read(dash_path)?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    let mut bus = MemoryBus::new();
    let vaddr_base: u64 = dash_xex.image_base as u64; // 0x92000000
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let uncompressed = dash_xex.decompress_image(&decrypted)?;
    let pe = PeImage::parse(&uncompressed)?;
    bus.load_bytes(vaddr_base, &uncompressed)?;
    println!("[+] Directly loaded {} bytes ({} MB) of virtual image at 0x{:08X}!", uncompressed.len(), uncompressed.len() / (1024*1024), vaddr_base);
    println!("[*] Dashboard 17559 PE Sections:");
    for sec in &pe.sections {
        println!("  - {:<10} VAddr: 0x{:08X} (size 0x{:08X}, RawPtr: 0x{:08X}, RawSize: 0x{:08X})",
            sec.name, sec.virtual_address, sec.virtual_size, sec.raw_data_ptr, sec.raw_data_size);
    }

    let mut thunk_map = dash_xex.setup_import_thunks(&mut bus)?;



    let init_val = bus.read_u32(0x9200312C).unwrap_or(0);
    println!("[*] Initial value at 0x9200312C = 0x{:08X}", init_val);

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

    let entry_vaddr = dash_xex.entry_point as u64; // 0x92196610
    println!("\n[*] Real Guest 17559 Entry Point: 0x{:08X}", entry_vaddr);

    println!("\n[*] Disassembling memmove 0x9219A200 - 0x9219A260:");
    for addr in (0x9219A200u64..0x9219A260).step_by(4) {
        let insn = bus.read_u32(addr).unwrap_or(0);
        println!("  [0x{:08X}] = 0x{:08X}", addr, insn);
    }

    let mut cpu = Cpu::new(entry_vaddr);
    cpu.regs.gpr[1] = stack_top;
    cpu.regs.gpr[3] = vaddr_base; // hModule
    cpu.regs.gpr[4] = 1;          // DLL_PROCESS_ATTACH
    cpu.regs.gpr[5] = 0;          // lpReserved
    cpu.regs.gpr[13] = kpcr_base; // KPCR
    cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top - 8, 0x80001000)?;
    bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;

    println!("\n[*] Disassembling 0x92806B00 - 0x92806B60:");
    for addr in (0x92806B00u64..0x92806B60).step_by(4) {
        let insn = bus.read_u32(addr).unwrap_or(0);
        println!("  [0x{:08X}] = 0x{:08X}", addr, insn);
    }



    println!("\n[*] Stepping PowerPC 64 CPU core through real 17559 Dashboard instructions...");
    let mut total_instructions = 0usize;
    let mut syscall_counts: HashMap<String, usize> = HashMap::new();
    let mut unique_pcs: HashSet<u64> = HashSet::new();

    let mut history: Vec<(usize, u64, u32, u64, u64)> = Vec::new();

    let mut last_count_312c = bus.read_u32(0x929613C0).unwrap_or(0);

    let mut last_buf_ptr = bus.read_u32(0x929613B8).unwrap_or(0);

    for step in 1..=200000 {
        let pc = cpu.regs.pc;
        let insn = bus.read_u32(pc).unwrap_or(0);
        unique_pcs.insert(pc);

        let cur_count = bus.read_u32(0x929613C0).unwrap_or(0);
        if cur_count != last_count_312c {
            println!("  [WATCHPOINT COUNT 0x929613C0] Step {} | PC: 0x{:08X} changed count from {} to {}!",
                step, pc, last_count_312c, cur_count);
            last_count_312c = cur_count;
        }

        let cur_buf = bus.read_u32(0x929613B8).unwrap_or(0);
        if cur_buf != last_buf_ptr {
            println!("  [WATCHPOINT BUF_PTR 0x929613B8] Step {} | PC: 0x{:08X} changed buf ptr from 0x{:08X} to 0x{:08X}!",
                step, pc, last_buf_ptr, cur_buf);
            last_buf_ptr = cur_buf;
        }

        if step == 60505 {
            println!("  [STRUCT 0x929613B8]:");
            for i in 0..6 {
                let off = 0x929613B8 + i * 4;
                println!("    +0x{:02X} (0x{:08X}) = 0x{:08X}", i * 4, off, bus.read_u32(off).unwrap_or(0));
            }
            println!("  [BUFFER 0x70003560]:");
            for i in 0..8 {
                let off = 0x70003560 + i * 4;
                println!("    +0x{:02X} (0x{:08X}) = 0x{:08X}", i * 4, off, bus.read_u32(off).unwrap_or(0));
            }
        }

        history.push((step, pc, insn, cpu.regs.lr, cpu.regs.ctr));
        if history.len() > 32 {
            history.remove(0);
        }

        if step >= 97300 && step <= 97375 {
            println!("  [CALLOC TRACE] Step {:05} | PC: 0x{:08X} | Insn: 0x{:08X} | r3=0x{:X}, r4=0x{:X}, r5=0x{:X}, r31=0x{:X}, LR=0x{:X}",
                step, pc, insn, cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5], cpu.regs.gpr[31], cpu.regs.lr);
        }

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

                    let args = [
                        cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5],
                        cpu.regs.gpr[6], cpu.regs.gpr[7], cpu.regs.gpr[8],
                    ];

                    if *ordinal == 0x0152 {
                        println!("    [TLS ALLOC] Step {} PC: 0x{:08X}, LR: 0x{:08X}", step, call_pc, cpu.regs.lr);
                    }
                    if *ordinal == 0x0155 && args[0] == 1 {
                        println!("    [TLS SET slot=1] Step {} val=0x{:X}, PC: 0x{:08X}, LR: 0x{:08X}", step, args[1], call_pc, cpu.regs.lr);
                    }
                    if *ordinal == 0x0154 && args[0] == 1 {
                        println!("    [TLS GET slot=1] Step {} PC: 0x{:08X}, LR: 0x{:08X}", step, call_pc, cpu.regs.lr);
                    }
                    let is_noisy = fname == "RtlEnterCriticalSection"
                        || fname == "RtlLeaveCriticalSection"
                        || fname == "KeTlsSetValue"
                        || fname == "KeTlsGetValue"
                        || fname == "KeGetCurrentProcessType";
                    if !is_noisy {
                        println!("    [SYSCALL] Step {} | -> {} (args: [0x{:X}, 0x{:X}, 0x{:X}, 0x{:X}])",
                            step, key, args[0], args[1], args[2], args[3]);
                    }

                    if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                        println!("  [DASH] DllMain ExTerminateThread completed at Step {} ({} insns)!", step, total_instructions);
                        break;
                    }

                    let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                    cpu.regs.gpr[3] = ret;
                    cpu.regs.pc = cpu.regs.lr;
                } else {
                    println!("  [FATAL] Syscall at PC 0x{:08X} not found in thunk map!", call_pc);
                    break;
                }
            }
            Ok(StepResult::Halted) => {
                println!("  [HALT] CPU halted at Step {}", step);
                break;
            }
            Err(e) => {
                println!("  [CPU ERROR] Step {} at PC 0x{:08X}: {:?}", step, pc, e);
                println!("    LR: 0x{:016X} | CTR: 0x{:016X}", cpu.regs.lr, cpu.regs.ctr);
                println!("    GPRs: r3=0x{:X}, r4=0x{:X}, r5=0x{:X}, r6=0x{:X}, r9=0x{:X}, r10=0x{:X}, r11=0x{:X}, r12=0x{:X}",
                    cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5], cpu.regs.gpr[6], cpu.regs.gpr[9], cpu.regs.gpr[10], cpu.regs.gpr[11], cpu.regs.gpr[12]);
                println!("    Recent instruction history before failure:");
                for (h_step, h_pc, h_insn, h_lr, h_ctr) in &history {
                    println!("      Step {:05} | PC: 0x{:08X} | Insn: 0x{:08X} | LR: 0x{:08X} | CTR: 0x{:08X}",
                        h_step, h_pc, h_insn, h_lr, h_ctr);
                }
                println!("    VR[0]: {:02X?}", &cpu.regs.vr[0]);
                if pc != 0 {
                    println!("  Disassembly around 0x{:08X}:", pc);
                    for a in (pc.saturating_sub(0x18)..=pc + 0x18).step_by(4) {
                        let insn = bus.read_u32(a).unwrap_or(0);
                        let m = if a == pc { "==>" } else { "   " };
                        println!("    {} [0x{:08X}] 0x{:08X}", m, a, insn);
                    }
                }
                break;
            }
        }
    }

    println!("\n========================================================");
    println!("  Summary of Dashboard 17559 Real Execution Trajectory   ");
    println!("========================================================");
    println!("  Total Instructions Executed: {}", total_instructions);
    println!("  Unique Instruction Addresses: {}", unique_pcs.len());
    println!("  Syscalls Intercepted & Dispatched: {}", syscall_counts.len());
    for (k, v) in &syscall_counts {
        println!("    - {:<45}: {} calls", k, v);
    }

    Ok(())
}
