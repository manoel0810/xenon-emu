pub mod cpu;
pub mod crypto;
pub mod memory;

use byteorder::{BigEndian, ByteOrder};
use cpu::Cpu;
use memory::MemoryBus;
use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xenon-Emu: Experimental Xbox 360 Emulator in Rust  ");
    println!("========================================================");

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

    // 1. Decrypt CB_A using the factory 1BL key
    println!("[*] Deriving CB_A key with 1BL HMAC-SHA1...");
    let cba_key = crypto::derive_cba_key(salt)?;
    println!("[+] Derived AES/RC4 Key: {:02X?}", cba_key);

    println!("[*] Decrypting bootloader payload with RC4 stream cipher...");
    let decrypted_cba = crypto::decrypt_bootloader_stage(&encrypted_cba, &cba_key);
    println!("[+] Bootloader payload decrypted successfully!");

    // 2. Initialize Memory Bus
    println!("[*] Initializing Memory Bus (512MB RAM + 512KB SRAM / Cache-As-RAM)...");
    let mut bus = MemoryBus::new();

    // 1BL loads CB_A into SRAM at address 0x8000_0200_0001_0000
    let sram_base: u64 = 0x8000_0200_0001_0000;
    bus.load_bytes(sram_base, &decrypted_cba)?;
    println!("[+] CB_A mapped to SRAM at: 0x{:016X}", sram_base);

    // 3. Initialize CPU at Entry Point (0x8000_0200_0001_03E0)
    let cpu_entry = sram_base + entry_point as u64;
    println!("[*] Initializing Xenon PowerPC 64 CPU at Entry Point: 0x{:016X}", cpu_entry);
    let mut cpu = Cpu::new(cpu_entry);

    println!("\n[*] Executing first bootloader instructions in Xenon-Emu CPU:");
    println!("------------------------------------------------------------");

    // Execute first 20 instructions
    for step in 1..=20 {
        let current_pc = cpu.regs.pc;
        let insn_raw = bus.read_u32(current_pc)?;
        println!(
            "\n[Step {:02}] PC: 0x{:016X} | Raw: 0x{:08X}",
            step, current_pc, insn_raw
        );

        match cpu.step(&mut bus) {
            Ok(()) => {
                cpu.regs.dump_key_registers();
            }
            Err(e) => {
                println!("[!] CPU Execution halted: {}", e);
                break;
            }
        }
    }

    println!("\n========================================================");
    println!("[+] Milestone 1: CPU execution loop running successfully!");
    println!("========================================================");

    Ok(())
}
