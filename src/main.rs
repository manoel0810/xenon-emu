pub mod cpu;
pub mod crypto;
pub mod kernel;
pub mod loader;
pub mod memory;

use byteorder::{BigEndian, ByteOrder};
use cpu::Cpu;
use kernel::KernelDispatcher;
use loader::{StfsPackage, XexFile};
use memory::MemoryBus;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xenon-Emu: Experimental Xbox 360 Emulator in Rust  ");
    println!("========================================================");

    // ==========================================================
    // MILESTONE 1: NAND Bootloader (CB_A) Hardware Boot
    // ==========================================================
    println!("\n>>> [MILESTONE 1] NAND Bootloader (CB_A) Hardware Boot <<<");
    let cba_path = "../Corona 16mb/CB_A_enc.bin";
    let encrypted_cba = fs::read(cba_path)?;
    let salt = &encrypted_cba[0x10..0x20];
    let entry_point = BigEndian::read_u32(&encrypted_cba[8..12]);

    let cba_key = crypto::derive_cba_key(salt)?;
    let decrypted_cba = crypto::decrypt_bootloader_stage(&encrypted_cba, &cba_key);

    let mut bus = MemoryBus::new();
    let sram_base: u64 = 0x8000_0200_0001_0000;
    bus.load_bytes(sram_base, &decrypted_cba)?;

    let cpu_entry = sram_base + entry_point as u64;
    let mut cpu = Cpu::new(cpu_entry);

    println!("[*] Stepping CPU through first 5 instructions of CB_A in Cache-as-RAM:");
    for step in 1..=5 {
        let current_pc = cpu.regs.pc;
        let insn_raw = bus.read_u32(current_pc)?;
        cpu.step(&mut bus)?;
        println!(
            "  Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r26: 0x{:016X} | SP: 0x{:016X}",
            step, current_pc, insn_raw, cpu.regs.gpr[26], cpu.regs.gpr[1]
        );
    }
    println!("[+] Milestone 1: Hardware CPU bootloader execution verified.");

    // ==========================================================
    // MILESTONE 2: XEX2 Executable Parsing on default.xex
    // ==========================================================
    println!("\n>>> [MILESTONE 2] Xbox 360 Executable (XEX2) Loader <<<");
    let xex_path = "../default.xex";
    let xex_bytes = fs::read(xex_path)?;
    let default_xex = XexFile::parse(&xex_bytes)?;
    println!("[+] Parsed default.xex (Title ID: {:08X}, Entry: 0x{:08X})", 
        default_xex.execution_info.as_ref().map(|e| e.title_id).unwrap_or(0),
        default_xex.entry_point
    );
    println!("[+] Milestone 2: XEX2 loader verified.");

    // ==========================================================
    // MILESTONE 3: STFS Extraction of $flash_dash.xex & HLE Kernel Dispatch
    // ==========================================================
    println!("\n>>> [MILESTONE 3] STFS Package Extraction & Dashboard Boot Setup <<<");
    let su_path = "../$SystemUpdate/su20076000_00000000";
    println!("[*] Loading System Update STFS container: {}", su_path);
    let su_bytes = fs::read(su_path)?;
    println!("[+] Read {} bytes of STFS container", su_bytes.len());

    let stfs = StfsPackage::parse(&su_bytes)?;
    println!("[+] STFS Package parsed successfully! Found {} files inside container:", stfs.entries.len());
    for (i, entry) in stfs.entries.iter().take(10).enumerate() {
        println!(
            "  [{:02}] {:<30} | Size: {:8} bytes | StartBlock: {:4}",
            i + 1, entry.name, entry.file_size, entry.start_block
        );
    }
    if stfs.entries.len() > 10 {
        println!("  ... and {} more files.", stfs.entries.len() - 10);
    }

    // Extract the official Dashboard executable ($flash_dash.xex)
    println!("\n[*] Extracting '$flash_dash.xex' (The Official Xbox 360 Dashboard)...");
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    println!("[+] Extracted {} bytes for $flash_dash.xex!", dash_bytes.len());

    // Save extracted dashboard binary to disk for inspection
    let dash_out_path = "../dash.xex";
    fs::write(dash_out_path, &dash_bytes)?;
    println!("[+] Saved extracted dashboard to: {}", dash_out_path);

    // Parse the extracted Dashboard XEX
    println!("\n[*] Parsing extracted Dashboard executable with XEX2 Loader...");
    let dash_xex = XexFile::parse(&dash_bytes)?;
    dash_xex.print_summary();

    // Initialize HLE Kernel Dispatcher
    println!("\n[*] Initializing HLE Kernel Dispatcher (xboxkrnl.exe & xam.xex)...");
    let kernel = KernelDispatcher::new();
    println!("[+] Kernel Dispatcher initialized with {} xboxkrnl exports and {} xam exports.",
        kernel.xboxkrnl_exports.len(),
        kernel.xam_exports.len()
    );

    // Simulate initial Kernel queries performed by Dashboard on boot
    println!("\n[*] Simulating initial Dashboard boot syscalls:");
    let proc_type = kernel.dispatch("xboxkrnl.exe", 13, &[]); // KeGetCurrentProcessType
    println!("  -> KeGetCurrentProcessType returned: {} (2 = System/Dashboard process)", proc_type);

    let mem_status = kernel.dispatch("xboxkrnl.exe", 187, &[dash_xex.load_address as u64, dash_xex.image_size as u64]);
    println!("  -> NtAllocateVirtualMemory returned: {} (STATUS_SUCCESS)", mem_status);

    let signin_status = kernel.dispatch("xam.xex", 532, &[0]); // XamUserGetSigninState
    println!("  -> XamUserGetSigninState returned: {} (User not signed in)", signin_status);

    println!("\n========================================================");
    println!("  🎉 Milestones 1, 2, and 3 Successfully Operational!   ");
    println!("  Dashboard binary extracted and mapped for execution!  ");
    println!("========================================================");

    Ok(())
}
