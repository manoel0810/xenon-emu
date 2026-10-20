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

    println!("\n[*] Scanning decompressed Dashboard PE image for XUIZ (XZP Packages):");
    let mut xzips = Vec::new();
    for i in 0..uncompressed.len().saturating_sub(4) {
        if &uncompressed[i..i + 4] == b"XUIZ" {
            xzips.push(i);
        }
    }
    println!("[+] Found {} XUIZ package(s) inside dash.xex at offsets: {:X?}", xzips.len(), xzips);

    use xenon_emu::loader::XzpPackage;
    let mut total_embedded_files = 0usize;
    for &off in &xzips {
        if off + 22 <= uncompressed.len() {
            if let Ok(xzp) = XzpPackage::parse(&uncompressed[off..]) {
                for entry in xzp.entries {
                    if !entry.data.is_empty() {
                        kernel.register_vfs_file(&entry.name, entry.data);
                        total_embedded_files += 1;
                    }
                }
            }
        }
    }
    println!("[+] Registered {} authentic assets from {} embedded XUIZ packages into Kernel VFS!", total_embedded_files, xzips.len());

    let mut disk_vfs_count = 0usize;
    if std::path::Path::new("../extracted_xui").exists() {
        fn load_dir_recursive(dir: &std::path::Path, kernel: &mut KernelDispatcher, count: &mut usize) {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        load_dir_recursive(&p, kernel, count);
                    } else if p.is_file() {
                        if let Ok(data) = std::fs::read(&p) {
                            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                                kernel.register_vfs_file(name, data);
                                *count += 1;
                            }
                        }
                    }
                }
            }
        }
        load_dir_recursive(std::path::Path::new("../extracted_xui"), &mut kernel, &mut disk_vfs_count);
        println!("[+] Registered {} additional assets from disk (../extracted_xui) into Kernel VFS!", disk_vfs_count);
    }
    println!("[+] Total VFS Files active in kernel namespace: {}", kernel.vfs_files.len());

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

    for step in 1..=1000000 {
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

        history.push((step, pc, insn, cpu.regs.lr, cpu.regs.ctr));
        if history.len() > 32 {
            history.remove(0);
        }

        if step % 25000 == 0 || (step >= 115030 && step <= 115060) {
            println!("  [HEARTBEAT] Step {:06} | PC: 0x{:08X} | LR: 0x{:08X} | CTR: 0x{:08X}",
                step, pc, cpu.regs.lr, cpu.regs.ctr);
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
                        cpu.regs.gpr[9], cpu.regs.gpr[10],
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

    println!("\n========================================================");
    println!("  Hardware & Framebuffer State Analysis                 ");
    println!("========================================================");

    let fb_front = 0x90314000u64;
    let fb_back = 0x906AC000u64;
    let fb_size = 0x398000usize; // 1280 * 720 * 4 = 3,686,400 bytes

    let mut front_non_zero = 0usize;
    let mut back_non_zero = 0usize;
    let mut front_sample = Vec::new();
    let mut back_sample = Vec::new();

    for i in 0..fb_size {
        let b = bus.read_u8(fb_front + i as u64).unwrap_or(0);
        if b != 0 {
            front_non_zero += 1;
            if front_sample.len() < 16 {
                front_sample.push((i, b));
            }
        }
        let b_back = bus.read_u8(fb_back + i as u64).unwrap_or(0);
        if b_back != 0 {
            back_non_zero += 1;
            if back_sample.len() < 16 {
                back_sample.push((i, b_back));
            }
        }
    }

    println!("[*] Frontbuffer @ 0x{:08X} ({} bytes):", fb_front, fb_size);
    println!("    - Non-zero bytes: {} / {} ({:.2}%)", front_non_zero, fb_size, (front_non_zero as f64 / fb_size as f64) * 100.0);
    if !front_sample.is_empty() {
        print!("    - First non-zero samples (offset: byte): ");
        for (off, val) in &front_sample {
            print!("[0x{:X}: 0x{:02X}] ", off, val);
        }
        println!();
    }

    println!("[*] Backbuffer @ 0x{:08X} ({} bytes):", fb_back, fb_size);
    println!("    - Non-zero bytes: {} / {} ({:.2}%)", back_non_zero, fb_size, (back_non_zero as f64 / fb_size as f64) * 100.0);
    if !back_sample.is_empty() {
        print!("    - First non-zero samples (offset: byte): ");
        for (off, val) in &back_sample {
            print!("[0x{:X}: 0x{:02X}] ", off, val);
        }
        println!();
    }

    // Inspect and decode PM4 packets in Ring Buffer
    let rb_addr = 0x90104000u64;
    println!("\n[*] Ring Buffer @ 0x{:08X}:", rb_addr);
    let mut rb_words = Vec::new();
    for i in (0..0x1000).step_by(4) {
        rb_words.push(bus.read_u32(rb_addr + i as u64).unwrap_or(0));
    }
    
    let mut idx = 0;
    while idx < rb_words.len() {
        let w = rb_words[idx];
        if w == 0 {
            idx += 1;
            continue;
        }
        let ptype = (w >> 30) & 3;
        match ptype {
            0 => {
                let base_reg = w & 0x7FFF;
                let count = (((w >> 16) & 0x3FFF) + 1) as usize;
                println!("  [PM4 Type 0] offset +0x{:04X} | Base Reg: 0x{:04X}, Count: {}", idx * 4, base_reg, count);
                for c in 0..count.min(8) {
                    if idx + 1 + c < rb_words.len() {
                        println!("      Reg 0x{:04X} = 0x{:08X}", base_reg + c as u32, rb_words[idx + 1 + c]);
                    }
                }
                idx += 1 + count;
            }
            1 => {
                let reg1 = w & 0x7FF;
                let reg2 = (w >> 11) & 0x7FF;
                println!("  [PM4 Type 1] offset +0x{:04X} | Reg1: 0x{:04X}, Reg2: 0x{:04X}", idx * 4, reg1, reg2);
                idx += 3;
            }
            2 => {
                println!("  [PM4 Type 2 NOP] offset +0x{:04X}", idx * 4);
                idx += 1;
            }
            3 => {
                let opcode = ((w >> 8) & 0xFF) as u8;
                let count = (((w >> 16) & 0x3FFF) + 1) as usize;
                let opname = match opcode {
                    0x10 => "NOP",
                    0x21 => "SET_CONSTANT",
                    0x22 => "SET_CONTEXT_REG",
                    0x23 => "DRAW_INDX",
                    0x24 => "DRAW_INDX_2",
                    0x2A => "INDEX_TYPE",
                    0x2F => "LOAD_ALU_CONSTANT",
                    0x34 => "WAIT_FOR_IDLE",
                    0x38 => "MEM_WRITE",
                    0x3F => "EXECUTE_INDIRECT",
                    0x48 => "ME_INIT",
                    0x4C => "SURFACE_SYNC",
                    _ => "UNKNOWN",
                };
                println!("  [PM4 Type 3] offset +0x{:04X} | Opcode: 0x{:02X} ({}) | Count: {}", idx * 4, opcode, opname, count);
                for c in 0..count.min(16) {
                    if idx + 1 + c < rb_words.len() {
                        println!("      Data[{}] = 0x{:08X}", c, rb_words[idx + 1 + c]);
                    }
                }
                idx += 1 + count;
            }
            _ => {
                println!("  [Raw Word] offset +0x{:04X} = 0x{:08X}", idx * 4, w);
                idx += 1;
            }
        }
    }

    // Inspect Indirect Command Buffers (IB) referenced by PM4_EXECUTE_INDIRECT
    let ib_addrs = [0x1011C060u64, 0x1010C000u64, 0x9011C060u64, 0x9010C000u64];
    for &ib in &ib_addrs {
        let first = bus.read_u32(ib).unwrap_or(0);
        println!("[*] Checking IB at 0x{:08X}: first word = 0x{:08X}", ib, first);
        if first != 0 {
            for k in 0..16 {
                let w = bus.read_u32(ib + (k * 4)).unwrap_or(0);
                let ptype = (w >> 30) & 3;
                let op = ((w >> 8) & 0xFF) as u8;
                let cnt = (((w >> 16) & 0x3FFF) + 1) as usize;
                println!("    [IB+0x{:02X}] 0x{:08X} (type={}, op=0x{:02X}, cnt={})", k * 4, w, ptype, op, cnt);
            }
        }
    }

    use xenon_emu::gpu::{RingBuffer, XenosGpu};
    let mut xenos_gpu = XenosGpu::new();
    let mut ring_buffer = RingBuffer::new(rb_addr, 0x10000);
    let mut last_non_zero = 0;
    for i in (0..0x10000).step_by(4) {
        if bus.read_u32(rb_addr + i as u64).unwrap_or(0) != 0 {
            last_non_zero = (i / 4) + 1;
        }
    }
    ring_buffer.wptr = last_non_zero;
    println!("\n[*] Executing Xenos GPU Command Processor on guest ring buffer (wptr = {} dwords)...", ring_buffer.wptr);
    match xenos_gpu.execute_ring(&mut bus, &mut ring_buffer) {
        Ok(n) => {
            println!("[+] Xenos GPU successfully executed {} PM4 packets from guest ring buffer!", n);
            println!("    - Total packets: {}", xenos_gpu.packets_executed);
            println!("    - Draw calls:    {}", xenos_gpu.draw_calls);
            println!("    - Primitives:    {}", xenos_gpu.primitives_drawn);
            println!("    - Registers set: {}", xenos_gpu.registers.len());
            for (reg, val) in &xenos_gpu.registers {
                println!("        Reg 0x{:04X} = 0x{:08X}", reg, val);
            }
        }
        Err(e) => {
            println!("[-] Xenos GPU execute_ring error: {}", e);
        }
    }

    // Save frontbuffer or backbuffer if non-zero
    let target_buffer = if back_non_zero > front_non_zero { fb_back } else { fb_front };
    let non_zero_count = front_non_zero.max(back_non_zero);
    if non_zero_count > 0 {
        let mut img_buf = image::RgbaImage::new(1280, 720);
        for y in 0..720 {
            for x in 0..1280 {
                let off = (y * 1280 + x) * 4;
                let b0 = bus.read_u8(target_buffer + off as u64).unwrap_or(0);
                let b1 = bus.read_u8(target_buffer + off as u64 + 1).unwrap_or(0);
                let b2 = bus.read_u8(target_buffer + off as u64 + 2).unwrap_or(0);
                let b3 = bus.read_u8(target_buffer + off as u64 + 3).unwrap_or(0);
                // Assume ARGB/BGRA or RGBA
                img_buf.put_pixel(x, y, image::Rgba([b1, b2, b3, b0]));
            }
        }
        let out_path = "dash_17559_framebuffer.png";
        img_buf.save(out_path)?;
        println!("[+] Saved 1280x720 framebuffer to {}", out_path);
    }

    // Save Xenos GPU internal color buffer
    let mut edram_non_zero = 0;
    for &px in &xenos_gpu.color_buffer {
        if px != 0 {
            edram_non_zero += 1;
        }
    }
    println!("[*] Xenos GPU Color Buffer non-zero pixels: {} / {}", edram_non_zero, xenos_gpu.color_buffer.len());
    if edram_non_zero > 0 {
        let mut edram_img = image::RgbaImage::new(1280, 720);
        for y in 0..720 {
            for x in 0..1280 {
                let px = xenos_gpu.color_buffer[y * 1280 + x];
                let a = ((px >> 24) & 0xFF) as u8;
                let r = ((px >> 16) & 0xFF) as u8;
                let g = ((px >> 8) & 0xFF) as u8;
                let b = (px & 0xFF) as u8;
                edram_img.put_pixel(x as u32, y as u32, image::Rgba([r, g, b, if a == 0 { 255 } else { a }]));
            }
        }
        let out_path = "dash_17559_gpu_edram.png";
        edram_img.save(out_path)?;
        println!("[+] Saved 1280x720 GPU EDRAM buffer to {}", out_path);
    }

    Ok(())
}
