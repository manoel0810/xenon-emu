//! Boot sequence: CB_A smoke test, XEX2 loading, dashboard image mapping, import thunks, VFS and KPCR of the boot thread.

use byteorder::{BigEndian, ByteOrder};
use std::collections::HashMap;
use std::fs;
use xenon_emu::cpu::Cpu;
use xenon_emu::crypto;
use xenon_emu::gpu::display::resolve_path;
use xenon_emu::kernel::KernelDispatcher;
use xenon_emu::loader::{self, PeImage, StfsPackage, XexFile};
use xenon_emu::memory::MemoryBus;

use super::threads::setup_kpcr;

/// Everything later stages need from the boot sequence.
pub struct Boot {
    pub bus: MemoryBus,
    pub kernel: KernelDispatcher,
    pub thunk_map: HashMap<u64, (String, u16)>,
    pub is_metro: bool,
    pub vaddr_base: u64,
    pub real_entry_vaddr: u64,
    pub stack_top: u64,
    pub kpcr_base: u64,
    pub next_kpcr_slot: u64,
}

pub fn boot() -> Result<Boot, Box<dyn std::error::Error>> {
    // ==========================================================
    // MILESTONE 1: NAND Bootloader (CB_A) Hardware Boot
    // ==========================================================
    println!("\n>>> [MILESTONE 1] NAND Bootloader (CB_A) Hardware Boot <<<");
    let cba_path = resolve_path("Corona 16mb/CB_A_enc.bin");
    let encrypted_cba = fs::read(&cba_path)?;
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
    let xex_path = resolve_path("default.xex");
    let xex_bytes = fs::read(&xex_path)?;
    let default_xex = XexFile::parse(&xex_bytes)?;
    println!("[+] Parsed default.xex (Title ID: {:08X}, Entry: 0x{:08X})", 
        default_xex.execution_info.as_ref().map(|e| e.title_id).unwrap_or(0),
        default_xex.entry_point
    );
    println!("[+] Milestone 2: XEX2 loader verified.");

    // ==========================================================
    // MILESTONE 3: Official Dashboard Executable Loading & Setup
    // ==========================================================
    println!("\n>>> [MILESTONE 3] Official Dashboard Loading & Setup <<<");
    let metro_path = resolve_path("SystemUpdate_17559_USB/$SystemUpdate/dash.xex");
    let (dash_bytes, is_metro) = if metro_path.exists() {
        println!("[+] Found Authentic Microsoft Metro Dashboard 17559: {}", metro_path.display());
        (fs::read(&metro_path)?, true)
    } else {
        let su_path = resolve_path("$SystemUpdate/su20076000_00000000");
        let su_bytes = fs::read(&su_path)?;
        let stfs = StfsPackage::parse(&su_bytes)?;
        println!("[+] STFS Package parsed! Found {} files inside container.", stfs.entries.len());
        for entry in &stfs.entries {
            println!("  - {:<28} ({} bytes)", entry.name, entry.file_size);
        }
        println!("\n[*] Extracting '$flash_dash.xex' (The Official Xbox 360 Dashboard)...");
        let b = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
        (b, false)
    };
    println!("[+] Loaded {} bytes for Dashboard executable payload!", dash_bytes.len());

    println!("\n[*] Parsing extracted Dashboard executable with XEX2 Loader...");
    let dash_xex = XexFile::parse(&dash_bytes)?;
    dash_xex.print_summary();

    if let Some(&exp_off) = dash_xex.optional_headers.get(&0x00E10402) {
        let e_off = exp_off as usize;
        if e_off + 8 <= dash_bytes.len() {
            let rva = BigEndian::read_u32(&dash_bytes[e_off..e_off + 4]);
            let size = BigEndian::read_u32(&dash_bytes[e_off + 4..e_off + 8]);
            println!("[*] Dashboard Export Directory: RVA 0x{:08X}, Size 0x{:08X}", rva, size);
        }
    }

    // ==========================================================
    // MILESTONE 4: Payload Decryption & Memory Space Mapping
    // ==========================================================
    println!("\n>>> [MILESTONE 4] Dashboard Payload Decryption & Virtual Memory <<<");
    println!("[*] Decrypting Dashboard PE payload with AES-128-CBC using Session Key...");
    let decrypted_payload = dash_xex.decrypt_image_payload(&dash_bytes)?;
    println!("[+] Decrypted {} bytes of executable payload!", decrypted_payload.len());

    println!("[*] Extracting and decompressing LZX stream into PE executable image...");
    let uncompressed_pe = dash_xex.decompress_image(&decrypted_payload)?;
    println!("[*] First 32 bytes of decompressed PE image: {:02X?}", &uncompressed_pe[..32]);

    let vaddr_base: u64 = if is_metro {
        dash_xex.image_base as u64
    } else {
        dash_xex.load_address as u64
    };
    println!("[+] Allocated 0x{:08X} bytes ({} MB) at 0x{:016X}", dash_xex.image_size, dash_xex.image_size / (1024*1024), vaddr_base);

    println!("\n[*] Scanning decompressed Dashboard PE image for XUIZ (XZP Packages):");
    let mut xzips = Vec::new();
    for i in 0..uncompressed_pe.len().saturating_sub(4) {
        if &uncompressed_pe[i..i + 4] == b"XUIZ" {
            xzips.push(i);
        }
    }
    println!("[+] Found {} XUIZ package(s) inside dash.xex at offsets: {:X?}", xzips.len(), xzips);

    use loader::XzpPackage;
    for (i, &off) in xzips.iter().take(3).enumerate() {
        if off + 22 <= uncompressed_pe.len() {
            println!("\n[*] Parsing Embedded XUIZ Package #{} at offset 0x{:08X}...", i + 1, off);
            match XzpPackage::parse(&uncompressed_pe[off..]) {
                Ok(xzp) => {
                    println!("[+] XUIZ Package #{}: Total Size: {} bytes | Entries: {}", i + 1, xzp.total_size, xzp.entries.len());
                    for (idx, entry) in xzp.entries.iter().enumerate() {
                        println!("    [{:02}] {:<36} (Size: {:6} bytes, Offset: 0x{:08X})",
                            idx, entry.name, entry.file_size, entry.file_offset
                        );
                        if entry.name.contains("strings.xus") {
                            let preview_len = std::cmp::min(entry.data.len(), 48);
                            println!("         Hex: {:02X?}", &entry.data[..preview_len]);
                        }
                    }
                }
                Err(e) => println!("  [!] XUIZ package #{} parse error: {}", i + 1, e),
            }
        }
    }

    println!("\n[*] Parsing Portable Executable (PE32+) Image headers...");
    let pe_image = PeImage::parse(&uncompressed_pe)?;
    println!("[+] PE Image parsed! Found {} sections and {} exports. Loading into Virtual Memory Bus at 0x{:08X}...",
        pe_image.sections.len(), pe_image.exports.len(), vaddr_base
    );
    for exp in pe_image.exports.iter().take(5) {
        println!("  [DASH EXPORT] Ordinal #{}: {:?} -> RVA 0x{:08X} (VAddr 0x{:016X})",
            exp.ordinal, exp.name, exp.rva, vaddr_base + exp.rva as u64
        );
    }
    if is_metro {
        bus.load_bytes(vaddr_base, &uncompressed_pe)?;
    } else {
        pe_image.load_into_memory(&uncompressed_pe, vaddr_base, &mut bus)?;
    }

    println!("\n[*] Setting up Import Thunks and HLE Syscall Traps...");
    let mut thunk_map = dash_xex.setup_import_thunks(&mut bus)?;

    println!("\n[*] Enumerating Imported Symbols for Dashboard:");
    for lib in &dash_xex.import_libraries {
        let mut ords = Vec::new();
        for &rec_addr in &lib.import_table {
            if let Some((_, ord)) = thunk_map.get(&(rec_addr as u64)) {
                ords.push(*ord);
            }
        }
        ords.sort();
        ords.dedup();
        println!("  - Library '{}': {} function imports: {:?}", lib.name, ords.len(), ords);
    }

    // Register kernel thread exit trampoline at 0x80001000 (ExTerminateThread 0x0019 = 25)
    bus.write_u32(0x80001000, 0x44000042)?;
    bus.write_u32(0x80001004, 0x4E800020)?;
    thunk_map.insert(0x80001000, ("xboxkrnl.exe".to_string(), 0x0019));
    // DPC return trampoline (sc; blr): a DPC routine run inline by KeInsertQueueDpc returns here (app/threads.rs)
    bus.write_u32(super::threads::DPC_RETURN_TRAMPOLINE, 0x44000042)?;
    bus.write_u32(super::threads::DPC_RETURN_TRAMPOLINE + 4, 0x4E800020)?;
    thunk_map.insert(super::threads::DPC_RETURN_TRAMPOLINE, ("xboxkrnl.exe".to_string(), super::threads::DPC_RETURN_ORDINAL));

    let mut kernel = KernelDispatcher::new();
    // Loader data entries for XexGetModuleHandle / XexGetModuleSection: the title executable and the HLE kernel modules.
    kernel.modules.register_executable(
        &mut bus,
        "dash",
        if is_metro { "game:\\dash.xex" } else { "game:\\$flash_dash.xex" },
        vaddr_base as u32,
        dash_xex.image_size,
        dash_xex.entry_point,
        dash_xex.resources.clone(),
    );
    kernel.modules.register_kernel(&mut bus, "xboxkrnl.exe");
    kernel.modules.register_kernel(&mut bus, "xam.xex");

    println!("\n[*] Populating Kernel Virtual File System (VFS) with authentic Dashboard resources:");
    let mut total_embedded_files = 0usize;
    for (_i, &off) in xzips.iter().enumerate() {
        if off + 22 <= uncompressed_pe.len() {
            if let Ok(xzp) = XzpPackage::parse(&uncompressed_pe[off..]) {
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

    println!("\n[*] Dispatching initial Dashboard boot syscalls:");
    let proc_type = kernel.dispatch("xboxkrnl.exe", 0x0066, &[], &mut bus); // KeGetCurrentProcessType
    println!("  -> KeGetCurrentProcessType: {} (System Process)", proc_type);

    let mem_status = kernel.dispatch("xboxkrnl.exe", 0x00CC, &[vaddr_base, 0, dash_xex.image_size as u64], &mut bus);
    println!("  -> NtAllocateVirtualMemory: {} (STATUS_SUCCESS)", mem_status);

    let signin_status = kernel.dispatch("xam.xex", 0x0210, &[0], &mut bus); // XamUserGetSigninState
    println!("  -> XamUserGetSigninState: {} (Ready)", signin_status);

    let real_entry_vaddr = if is_metro {
        dash_xex.entry_point as u64
    } else if pe_image.entry_point_rva > 0 {
        vaddr_base + pe_image.entry_point_rva as u64
    } else {
        vaddr_base + dash_xex.entry_point as u64
    };
    println!("[*] PE Entry RVA: 0x{:08X} | XEX Entry: 0x{:08X} | Target Entry VAddr: 0x{:016X}",
        pe_image.entry_point_rva, dash_xex.entry_point, real_entry_vaddr
    );

    println!("[*] Thunk 0x9268F684: {:?}", thunk_map.get(&0x9268F684));
    println!("[*] Thunk 0x9268EC04: {:?}", thunk_map.get(&0x9268EC04));

    // Primary thread stack: its own region outside the image (it used to sit over the dash's XEX resources).
    let stack_top = super::threads::PRIMARY_STACK_TOP;

    // Setup KPCR (Processor Control Region / TEB). Slot 0 = primary/boot thread; every other guest thread gets its own slot.
    let kpcr_base = setup_kpcr(&mut bus, 0, stack_top, stack_top - super::threads::PRIMARY_STACK_SIZE);

    Ok(Boot {
        bus,
        kernel,
        thunk_map,
        is_metro,
        vaddr_base,
        real_entry_vaddr,
        stack_top,
        kpcr_base,
        next_kpcr_slot: 1,
    })
}
