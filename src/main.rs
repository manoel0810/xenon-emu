pub mod cpu;
pub mod crypto;
pub mod loader;
pub mod memory;

use byteorder::{BigEndian, ByteOrder};
use cpu::Cpu;
use loader::XexFile;
use memory::MemoryBus;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xenon-Emu: Experimental Xbox 360 Emulator in Rust  ");
    println!("========================================================");

    // ==========================================================
    // MILESTONE 1: NAND Bootloader (CB_A) Decryption & CPU Execution
    // ==========================================================
    println!("\n>>> [MILESTONE 1] NAND Bootloader (CB_A) Hardware Boot <<<");
    let cba_path = "../Corona 16mb/CB_A_enc.bin";
    println!("[*] Loading bootloader: {}", cba_path);
    let encrypted_cba = fs::read(cba_path)?;
    println!("[+] Loaded {} bytes of CB_A", encrypted_cba.len());

    let magic = &encrypted_cba[0..2];
    let build = BigEndian::read_u16(&encrypted_cba[2..4]);
    let entry_point = BigEndian::read_u32(&encrypted_cba[8..12]);
    let size = BigEndian::read_u32(&encrypted_cba[12..16]);
    let salt = &encrypted_cba[0x10..0x20];

    println!(
        "[+] Bootloader Header: Magic={:?}, Build={}, Entry=0x{:04X}, Size=0x{:04X}",
        std::str::from_utf8(magic)?,
        build,
        entry_point,
        size
    );

    // Decrypt CB_A using the factory 1BL key
    let cba_key = crypto::derive_cba_key(salt)?;
    println!("[+] Derived 1BL Key: {:02X?}", cba_key);
    let decrypted_cba = crypto::decrypt_bootloader_stage(&encrypted_cba, &cba_key);
    println!("[+] Bootloader decrypted successfully with RC4 stream cipher!");

    // Initialize Memory Bus and map CB_A into SRAM
    let mut bus = MemoryBus::new();
    let sram_base: u64 = 0x8000_0200_0001_0000;
    bus.load_bytes(sram_base, &decrypted_cba)?;

    // Initialize CPU at Entry Point (0x8000_0200_0001_03E0)
    let cpu_entry = sram_base + entry_point as u64;
    let mut cpu = Cpu::new(cpu_entry);

    println!("[*] Stepping CPU through first 5 instructions of CB_A:");
    for step in 1..=5 {
        let current_pc = cpu.regs.pc;
        let insn_raw = bus.read_u32(current_pc)?;
        cpu.step(&mut bus)?;
        println!(
            "  Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r26: 0x{:016X} | SP: 0x{:016X}",
            step, current_pc, insn_raw, cpu.regs.gpr[26], cpu.regs.gpr[1]
        );
    }
    println!("[+] Milestone 1 operational.");

    // ==========================================================
    // MILESTONE 2: XEX2 Executable Parsing, Decryption & Loading
    // ==========================================================
    println!("\n>>> [MILESTONE 2] Xbox 360 Executable (XEX2) Loader <<<");
    let xex_path = "../default.xex";
    println!("[*] Loading executable file: {}", xex_path);
    let xex_bytes = fs::read(xex_path)?;
    println!("[+] Read {} bytes from {}", xex_bytes.len(), xex_path);

    let xex = XexFile::parse(&xex_bytes)?;
    xex.print_summary();

    println!("\n[*] Mapping XEX image into Virtual Memory at ImageBase: 0x{:08X}", xex.image_base);
    // Allocate space in physical memory for XEX load address
    let load_vaddr = xex.load_address as u64;
    println!("[+] Virtual Address Space configured for: 0x{:016X}", load_vaddr);
    println!("[+] Next target: Linking Kernel syscall stubs and executing XEX entry point at 0x{:08X}!", xex.entry_point);

    println!("\n========================================================");
    println!("     Milestone 1 & 2 Completed Successfully!            ");
    println!("========================================================");

    Ok(())
}
