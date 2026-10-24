use xenon_emu::cpu::{Cpu, StepResult};
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::XexFile;
use xenon_emu::memory::MemoryBus;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dash_path = "../SystemUpdate_17559_USB/$SystemUpdate/dash.xex";
    let dash_bytes = fs::read(dash_path)?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    let mut bus = MemoryBus::new();
    let vaddr_base: u64 = dash_xex.image_base as u64;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let uncompressed = dash_xex.decompress_image(&decrypted)?;
    bus.load_bytes(vaddr_base, &uncompressed)?;
    let mut thunk_map = dash_xex.setup_import_thunks(&mut bus)?;

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
    let mut cpu = Cpu::new(dash_xex.entry_point as u64);
    cpu.regs.gpr[1] = stack_top;
    cpu.regs.gpr[13] = kpcr_base;
    cpu.regs.lr = 0x80001000;

    // Run 300,000 steps until VdSetGraphicsInterruptCallback is set
    for _ in 0..300_000 {
        match cpu.step(&mut bus) {
            Ok(StepResult::Ok) => {}
            Ok(StepResult::Syscall { pc, .. }) => {
                let (lib, ord) = thunk_map.get(&pc).cloned().unwrap_or(("unknown".to_string(), 0));
                let args = [
                    cpu.regs.gpr[3], cpu.regs.gpr[4], cpu.regs.gpr[5], cpu.regs.gpr[6],
                    cpu.regs.gpr[7], cpu.regs.gpr[8], cpu.regs.gpr[9], cpu.regs.gpr[10],
                ];
                let ret = kernel.dispatch(&lib, ord, &args, &mut bus);
                cpu.regs.gpr[3] = ret;
                cpu.regs.pc = cpu.regs.lr;
            }
            _ => break,
        }
    }

    println!("[*] Graphics Interrupt Callback registered: 0x{:08X}", kernel.graphics_interrupt_callback);
    println!("[*] Graphics Interrupt Context:    0x{:08X}", kernel.graphics_interrupt_context);

    if kernel.graphics_interrupt_callback != 0 {
        let cb = kernel.graphics_interrupt_callback;
        println!("[*] Disassembling first 10 instructions of callback at 0x{:08X}:", cb);
        for i in 0..10 {
            let addr = cb + (i * 4);
            let insn = bus.read_u32(addr)?;
            println!("  0x{:08X}: 0x{:08X}", addr, insn);
        }

        // Test executing the callback!
        let cb_stack = stack_top - 0x1E0000;
        let mut cb_cpu = Cpu::new(cb);
        cb_cpu.regs.gpr[1] = cb_stack;
        cb_cpu.regs.gpr[3] = kernel.graphics_interrupt_context;
        cb_cpu.regs.gpr[4] = 1; // VBLANK
        cb_cpu.regs.gpr[5] = 1; // frame 1
        cb_cpu.regs.gpr[13] = kpcr_base;
        cb_cpu.regs.lr = 0x80001000;

        println!("[*] Executing VBlank callback for up to 5,000 steps...");
        let mut steps = 0;
        let mut syscalls = Vec::new();
        for _ in 0..5000 {
            steps += 1;
            match cb_cpu.step(&mut bus) {
                Ok(StepResult::Ok) => {}
                Ok(StepResult::Syscall { pc, .. }) => {
                    let (lib, ord) = thunk_map.get(&pc).cloned().unwrap_or(("unknown".to_string(), 0));
                    syscalls.push((lib.clone(), ord));
                    let args = [
                        cb_cpu.regs.gpr[3], cb_cpu.regs.gpr[4], cb_cpu.regs.gpr[5], cb_cpu.regs.gpr[6],
                        cb_cpu.regs.gpr[7], cb_cpu.regs.gpr[8], cb_cpu.regs.gpr[9], cb_cpu.regs.gpr[10],
                    ];
                    let ret = kernel.dispatch(&lib, ord, &args, &mut bus);
                    cb_cpu.regs.gpr[3] = ret;
                    cb_cpu.regs.pc = cb_cpu.regs.lr;
                }
                _ => {
                    break;
                }
            }
            if cb_cpu.regs.pc == 0x80001000 {
                println!("[+] Callback returned gracefully to ExTerminateThread in {} steps!", steps);
                break;
            }
        }
        println!("[*] Syscalls made during callback: {:?}", syscalls);
    }

    Ok(())
}
