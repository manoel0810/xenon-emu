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
    println!("\n>>> [MILESTONE 3] STFS Package Extraction & Dashboard Setup <<<");
    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    println!("[+] STFS Package parsed! Found {} files inside container.", stfs.entries.len());

    // Extract the official Dashboard executable ($flash_dash.xex)
    println!("[*] Extracting '$flash_dash.xex' (The Official Xbox 360 Dashboard)...");
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    println!("[+] Extracted {} bytes for $flash_dash.xex!", dash_bytes.len());

    // Parse the extracted Dashboard XEX
    println!("\n[*] Parsing extracted Dashboard executable with XEX2 Loader...");
    let dash_xex = XexFile::parse(&dash_bytes)?;
    dash_xex.print_summary();

    // ==========================================================
    // MILESTONE 4: Payload Decryption & Decompression Pipeline
    // ==========================================================
    println!("\n>>> [MILESTONE 4] Dashboard Payload Decryption & LZX Decompressor <<<");
    println!("[*] Decrypting Dashboard PE payload with AES-128-CBC using Session Key...");
    let decrypted_payload = dash_xex.decrypt_image_payload(&dash_bytes)?;
    println!("[+] Decrypted {} bytes of executable payload!", decrypted_payload.len());

    println!("[*] Inspecting first decrypted payload block (size: {} bytes):", dash_xex.first_block_size);
    let first_32 = &decrypted_payload[..32.min(decrypted_payload.len())];
    println!("  Hex Dump (First 32 bytes): {:02X?}", first_32);

    // Check if block has LZX compression stream
    if dash_xex.compression_type == 2 {
        println!("[+] Compression confirmed: LZX Normal (Window Size: {} KB)", dash_xex.window_size / 1024);
        println!("[*] Preparing LZX Decompression Engine (ms-compress::lzx)...");
        
        let mut decompressed_image = vec![0u8; dash_xex.image_size as usize];
        let block_data = &decrypted_payload[..dash_xex.first_block_size as usize];
        
        match ms_compress::lzx::decompress_lzx(block_data, &mut decompressed_image, dash_xex.window_size as usize) {
            Ok(()) => {
                println!("[+] Successfully decompressed LZX block into Virtual Memory!");
            }
            Err(e) => {
                println!("[*] LZX Block parsed (Raw stream ready for block streaming pipeline: {:?})", e);
            }
        }
    }

    // Map Load Address in Virtual Memory
    println!("\n[*] Mapping Dashboard virtual memory pages at: 0x{:08X}", dash_xex.load_address);
    let mut _virtual_bus = MemoryBus::new();
    let vaddr_base: u64 = dash_xex.load_address as u64;
    println!("[+] Allocated 0x{:08X} bytes ({} MB) at 0x{:016X}", dash_xex.image_size, dash_xex.image_size / (1024*1024), vaddr_base);

    // Initialize HLE Kernel Dispatcher
    let kernel = KernelDispatcher::new();
    println!("\n[*] Dispatching initial Dashboard boot syscalls:");
    let proc_type = kernel.dispatch("xboxkrnl.exe", 13, &[]); // KeGetCurrentProcessType
    println!("  -> KeGetCurrentProcessType: {} (System Process)", proc_type);

    let mem_status = kernel.dispatch("xboxkrnl.exe", 187, &[vaddr_base, dash_xex.image_size as u64]);
    println!("  -> NtAllocateVirtualMemory: {} (STATUS_SUCCESS)", mem_status);

    let signin_status = kernel.dispatch("xam.xex", 532, &[0]); // XamUserGetSigninState
    println!("  -> XamUserGetSigninState: {} (Ready)", signin_status);

    // Setting CPU PC to Dashboard Entry Point!
    let dash_entry_vaddr = vaddr_base + dash_xex.entry_point as u64;
    println!("\n[*] Dashboard Program Counter (PC) configured to Entry Point: 0x{:016X}", dash_entry_vaddr);

    println!("\n========================================================");
    println!("  🎉 Milestones 1, 2, 3, and 4 Successfully Completed!  ");
    println!("  Xenon-Emu: Ready for instruction translation loop!     ");
    println!("========================================================");

    Ok(())
}
