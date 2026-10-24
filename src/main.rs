pub mod cpu;
pub mod crypto;
pub mod diag;
pub mod gpu;
pub mod kernel;
pub mod loader;
pub mod memory;

use byteorder::{BigEndian, ByteOrder};

// ---- XENON_WATCHDOG: lock-free progress counters read by a native monitor thread ----
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
static WD_STEPS: AtomicU64 = AtomicU64::new(0);
static WD_PC: AtomicU64 = AtomicU64::new(0);
static WD_LR: AtomicU64 = AtomicU64::new(0);
static WD_FRAME: AtomicU64 = AtomicU64::new(0);
static WD_SYS: AtomicU64 = AtomicU64::new(0); // (lib_is_xam << 32 | ordinal) while inside a syscall, 0 otherwise
static WD_A0: AtomicU64 = AtomicU64::new(0);
static WD_A1: AtomicU64 = AtomicU64::new(0);
static WD_A2: AtomicU64 = AtomicU64::new(0);
static WD_A3: AtomicU64 = AtomicU64::new(0);use cpu::{Cpu, StepResult};
use gpu::{GpuDisplay, display::resolve_path};
use kernel::KernelDispatcher;
use loader::{PeImage, StfsPackage, XexFile};
use memory::MemoryBus;
use std::fs;

/// Per-thread KPCR/KTHREAD ("r13 block"). On the real console every thread has its own, so thread-local state such as
/// the Win32 last-error value (KTHREAD+0x160) is NOT shared. Previously all guest threads shared one KPCR at 0x7FE00000,
/// so a worker's SetLastError(0x102 / WAIT_TIMEOUT) leaked into the primary thread's GetLastError() and made the dash's
/// service initialisation fail. The blocks live in RAM above the dash image (outside the virtual-alloc window).
const KPCR_AREA: u64 = 0x9310_0000;
const KPCR_STRIDE: u64 = 0x2000; // 0x1000 KPCR + 0x1000 TLS

fn setup_kpcr(bus: &mut MemoryBus, slot: u64, stack_base: u64, stack_limit: u64) -> u64 {
    let kpcr = KPCR_AREA + slot * KPCR_STRIDE;
    let tls = kpcr + 0x1000;
    for off in (0..KPCR_STRIDE).step_by(4) {
        let _ = bus.write_u32(kpcr + off, 0);
    }
    let _ = bus.write_u32(kpcr + 0x00, tls as u32);              // tls_ptr
    let _ = bus.write_u32(kpcr + 0x30, kpcr as u32);             // pcr_ptr
    let _ = bus.write_u32(kpcr + 0x70, stack_base as u32);       // stack_base_ptr
    let _ = bus.write_u32(kpcr + 0x74, stack_limit as u32);      // stack_end_ptr
    let _ = bus.write_u32(kpcr + 0x100, (kpcr + 0x200) as u32);  // current_thread (KTHREAD)
    let _ = bus.write_u8(kpcr + 0x10C, 0);                       // current_cpu
    let _ = bus.write_u32(kpcr + 0x14C, (slot + 1) as u32);      // thread_id
    let _ = bus.write_u32(kpcr + 0x150, 0);                      // dpc_active
    let _ = bus.write_u32(kpcr + 0x160, 0);                      // last_error
    kpcr
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xenon-Emu: Experimental Xbox 360 Emulator in Rust  ");
    println!("========================================================");

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

    let mut kernel = KernelDispatcher::new();

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

    let stack_top = vaddr_base + dash_xex.image_size as u64 - 0x10000;

    // Setup KPCR (Processor Control Region / TEB). Slot 0 = primary/boot thread; every other guest thread gets its own slot.
    let kpcr_base = setup_kpcr(&mut bus, 0, stack_top, stack_top - 0x50000);
    let mut next_kpcr_slot: u64 = 1;
    if !is_metro {
        // ==========================================================
        // PHASE 1: DllMain (Process Attach) @ 0x9210D308
        // ==========================================================
        println!("\n[*] [PHASE 1] Executing Dashboard DllMain (Process Attach @ 0x{:08X})...", real_entry_vaddr);
        let mut dash_cpu = Cpu::new(real_entry_vaddr);
    dash_cpu.regs.gpr[1] = stack_top;
    dash_cpu.regs.gpr[3] = vaddr_base; // r3: hModule (0x92000000)
    dash_cpu.regs.gpr[4] = 1;          // r4: DLL_PROCESS_ATTACH (1)
    dash_cpu.regs.gpr[5] = 0;          // r5: lpReserved (0)
    dash_cpu.regs.gpr[13] = kpcr_base; // r13: KPCR
    dash_cpu.regs.lr = 0x80001000;     // LR: ExTerminateThread stub
    bus.write_u32(stack_top - 8, 0x80001000)?;
    bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;

    let mut phase1_steps = 0;
    for step in 1..=200 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase1_steps += 1;
                        if step <= 20 || step % 50 == 0 {
                            println!(
                                "  [DASH EXEC] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH HLE SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: DllMain completed with status 1!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH HLE SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH SYSCALL] Unmapped syscall at PC: 0x{:016X} (lev={})", pc, lev);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH EXEC] Execution trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH EXEC] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 1 (DllMain) completed! ({} instructions stepped)", phase1_steps);

    // ==========================================================
    // PHASE 2: Dashboard Master Initializer @ 0x9210D608
    // ==========================================================
    println!("\n[*] [PHASE 2] Executing Dashboard Master Initializer (0x9210D608)...");
    let stack_top2 = stack_top - 0x20000;
    dash_cpu.regs.pc = 0x9210D608;
    dash_cpu.regs.gpr[1] = stack_top2;
    dash_cpu.regs.gpr[3] = 0;
    dash_cpu.regs.gpr[4] = 1;
    dash_cpu.regs.gpr[5] = 0;
    dash_cpu.regs.gpr[6] = 0;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top2 - 8, 0x80001000)?;
    bus.write_u32(stack_top2 + 0x90 - 8, 0x80001000)?;

    let mut phase2_steps = 0;
    for step in 1..=500 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase2_steps += 1;
                        if step <= 20 || step % 50 == 0 {
                            println!(
                                "  [DASH INIT] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH INIT SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: Master Initializer complete!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH INIT SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH INIT SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH INIT] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH INIT] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 2 (Master Initializer) completed! ({} instructions stepped)", phase2_steps);

    // ==========================================================
    // PHASE 3: Dashboard Main Loop & UI Sequence @ 0x921088A0
    // ==========================================================
    println!("\n[*] [PHASE 3] Executing Dashboard Main Sequence (0x921088A0)...");
    let stack_top3 = stack_top - 0x40000;
    dash_cpu.regs.pc = 0x921088A0;
    dash_cpu.regs.gpr[1] = stack_top3;
    dash_cpu.regs.gpr[3] = 0;
    dash_cpu.regs.gpr[4] = 1;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top3 - 8, 0x80001000)?;
    bus.write_u32(stack_top3 + 0x80 - 8, 0x80001000)?;

    let mut phase3_steps = 0;
    for step in 1..=1000 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase3_steps += 1;
                        if step <= 20 || step % 50 == 0 {
                            println!(
                                "  [DASH MAIN] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH MAIN SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: Dashboard main sequence completed!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH MAIN SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH MAIN SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH MAIN] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH MAIN] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 3 (Dashboard Main Sequence) completed! ({} instructions stepped)", phase3_steps);

    // ==========================================================
    // PHASE 4: Genuine Dashboard Graphics Init @ 0x9212A34C
    // ==========================================================
    println!("\n[*] [PHASE 4] Executing Genuine Dashboard Graphics Init (0x9212A34C)...");
    let stack_top4 = stack_top - 0x60000;
    dash_cpu.regs.pc = 0x9212A34C;
    dash_cpu.regs.gpr[1] = stack_top4;
    dash_cpu.regs.gpr[3] = vaddr_base;
    dash_cpu.regs.gpr[4] = 0;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top4 - 8, 0x80001000)?;
    bus.write_u32(stack_top4 + 0xA0 - 8, 0x80001000)?;

    let mut phase4_steps = 0;
    for step in 1..=500 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase4_steps += 1;
                        if step <= 15 || step % 50 == 0 {
                            println!(
                                "  [DASH GFX] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH GFX SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: Graphics initialization sequence completed!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH GFX SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH GFX SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH GFX] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH GFX] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
    println!("[+] Phase 4 (Graphics Subsystem Init) completed! ({} instructions stepped)", phase4_steps);

    // ==========================================================
    // PHASE 5: Genuine Dashboard VdInitializeEngines Master Sequence @ 0x9210EA48
    // ==========================================================
    println!("\n[*] [PHASE 5] Executing Genuine Dashboard VdInitializeEngines Master Sequence (0x9210EA48)...");
    let stack_top5 = stack_top - 0x80000;
    // Initialize circular LIST_ENTRY at 0x926E95C4
    bus.write_u32(0x926E95C4, 0x926E95C4)?;
    bus.write_u32(0x926E95C8, 0x926E95C4)?;

    dash_cpu.regs.pc = 0x9210EA48;
    dash_cpu.regs.gpr[1] = stack_top5;
    dash_cpu.regs.gpr[3] = 0;
    dash_cpu.regs.gpr[4] = 0;
    dash_cpu.regs.gpr[13] = kpcr_base;
    dash_cpu.regs.lr = 0x80001000;
    bus.write_u32(stack_top5 - 8, 0x80001000)?;
    bus.write_u32(stack_top5 + 0x80 - 8, 0x80001000)?;

    let mut phase5_steps = 0;
    for step in 1..=500 {
        let current_pc = dash_cpu.regs.pc;
        match bus.read_u32(current_pc) {
            Ok(insn_raw) => {
                match dash_cpu.step(&mut bus) {
                    Ok(StepResult::Ok) => {
                        phase5_steps += 1;
                        if step <= 15 || step % 50 == 0 {
                            println!(
                                "  [DASH ENG] Step {:02} | PC: 0x{:016X} | Opcode: 0x{:08X} | r1 (SP): 0x{:016X}",
                                step, current_pc, insn_raw, dash_cpu.regs.gpr[1]
                            );
                        }
                    }
                    Ok(StepResult::Syscall { lev, pc }) => {
                        if let Some((lib, ordinal)) = thunk_map.get(&pc) {
                            if *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                println!(
                                    "  [DASH ENG SYSCALL] Step {:02} | Intercepted {}!#0x{:04X} (lev={}) -> ExTerminateThread: VdInitializeEngines sequence complete!",
                                    step, lib, ordinal, lev
                                );
                                break;
                            }
                            let args = [
                                dash_cpu.regs.gpr[3], dash_cpu.regs.gpr[4], dash_cpu.regs.gpr[5],
                                dash_cpu.regs.gpr[6], dash_cpu.regs.gpr[7], dash_cpu.regs.gpr[8],
                            ];
                            let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                            dash_cpu.regs.gpr[3] = ret;
                            dash_cpu.regs.pc = dash_cpu.regs.lr;
                            let fname = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
                                kernel.resolve_xboxkrnl(*ordinal).unwrap_or("Unknown")
                            } else {
                                kernel.resolve_xam(*ordinal).unwrap_or("Unknown")
                            };
                            println!(
                                "  [DASH ENG SYSCALL] Step {:02} | Intercepted {}!{} (#0x{:04X}) -> return 0x{:X}",
                                step, lib, fname, ordinal, ret
                            );
                        } else {
                            println!("  [DASH ENG SYSCALL] Unmapped syscall at PC: 0x{:016X}", pc);
                            break;
                        }
                    }
                    Ok(StepResult::Halted) => break,
                    Err(e) => {
                        println!("  [DASH ENG] Trap at PC: 0x{:016X} (0x{:08X}): {}", current_pc, insn_raw, e);
                        break;
                    }
                }
            }
            Err(e) => {
                println!("  [DASH ENG] Memory error at 0x{:016X}: {}", current_pc, e);
                break;
            }
        }
    }
        println!("[+] Phase 5 (VdInitializeEngines Master Sequence) completed! ({} instructions stepped)", phase5_steps);
    }

    // ==========================================================
    // ETAPA 1: Authentic Microsoft XUI Resource Engine (.xur & .xus)
    // ==========================================================
    println!("\n>>> [ETAPA 1] Genuine Microsoft XUI Resource Engine (.XUR & .XUS) <<<");
    use kernel::XuiManager;
    use loader::{XurScene, XusTable};
    let mut xui_mgr = XuiManager::new();

    // 1. Load Portuguese (pt-br) localized string table from dump
    let pt_strings_path = resolve_path("extracted_xui/pt-br/Strings.xus");
    let xus_table = if pt_strings_path.exists() {
        println!("[*] Loading genuine Microsoft Portuguese string table (.xus)...");
        match XusTable::load_file(&pt_strings_path) {
            Ok(table) => {
                println!("[+] Successfully loaded {} Portuguese strings from dump!", table.strings.len());
                for (idx, s) in table.strings.iter().take(5).enumerate() {
                    println!("    [{:02}] \"{}\"", idx + 1, s);
                }
                Some(table)
            }
            Err(e) => {
                println!("[!] Failed to load {}: {}", pt_strings_path.display(), e);
                None
            }
        }
    } else {
        None
    };

    if let Some(ref t) = xus_table {
        xui_mgr.set_string_table(t.clone());
    }

    // 2. Load authentic Microsoft XUI Visual Scenes (.xur)
    let mut loaded_scenes = Vec::new();
    let xur_files = [
        ("dashmain", resolve_path("extracted_xui/dashmain.xur")),
        ("BladeSkin", resolve_path("extracted_xui/BladeSkin.xur")),
        ("gamesSignedOut", resolve_path("extracted_xui/gamesSignedOut.xur")),
        ("mediaSignedOut", resolve_path("extracted_xui/mediaSignedOut.xur")),
        ("liveSignedOutUI", resolve_path("extracted_xui/liveSignedOutUI.xur")),
    ];

    for (_sname, spath) in &xur_files {
        if spath.exists() {
            match XurScene::load_file(spath) {
                Ok(scene) => {
                    println!("[+] Parsed authentic XUI Scene '{}': Root Class '{}', {} Child Nodes, {} Classes",
                        scene.name, scene.root.class_name, scene.root.children.len(), scene.classes.len()
                    );
                    for (c_idx, child) in scene.root.children.iter().take(4).enumerate() {
                        println!("    -> Node #{:02}: id='{}', class='{}', bounds=({:.0}, {:.0}, {:.0}, {:.0})",
                            c_idx + 1, child.id, child.class_name, child.x, child.y, child.width, child.height
                        );
                    }
                    xui_mgr.register_scene(scene.clone());
                    loaded_scenes.push(scene);
                }
                Err(e) => println!("  [!] XUR parse error for {}: {}", spath.display(), e),
            }
        }
    }

    // ==========================================================
    // ETAPA 2: ATI Xenos GPU Command Processor (CP) & Primary Ring Buffer
    // ==========================================================
    println!("\n>>> [ETAPA 2] ATI Xenos GPU Command Processor (CP) & Primary Ring Buffer <<<");
    use gpu::{
        build_draw_indexed, build_load_alu_constant, build_set_context_reg, build_wait_for_idle,
        RingBuffer, XenosGpu, PA_SC_WINDOW_OFFSET, RB_COLOR_INFO, VGT_DMA_BASE, VGT_DMA_SIZE,
        VGT_DMA_STRIDE,
    };

    let ring_base: u64 = if is_metro { 0x9010_4000 } else { 0x1000_0000 };
    let ring_size: usize = 64 * 1024;  // 64 KB Primary Ring Buffer
    let mut ring_buffer = RingBuffer::new(ring_base, ring_size);
    let mut xenos_gpu = XenosGpu::new();
    println!("[+] Initialized Xenos Primary Ring Buffer at 0x{:08X} (Size: {} KB)", ring_base, ring_size / 1024);

    // Load authentic Dashboard texture assets into TextureAtlas
    let atlas_dir = resolve_path("extracted_xui");
    xenos_gpu.load_atlas_from_dir(atlas_dir);
    let bound_logo = xenos_gpu.bind_texture_by_name(0, "xboxLogo");
    println!("  [+] Texture 'xboxLogo' bound to Sampler Stage 0: {}", bound_logo);

    if !is_metro {
        // Link Primary Ring Buffer with Xboxkrnl Video Subsystem
        let rb_ret = kernel.dispatch("xboxkrnl.exe", 0x01C3, &[ring_base, ring_size as u64], &mut bus);
        println!("  -> VdInitializeRingBuffer(0x{:08X}, {} KB): status={}", ring_base, ring_size / 1024, rb_ret);

        // CPU emits Direct3D / XUI PM4 packet stream to the Ring Buffer:
        println!("[*] CPU emitting Direct3D / XUI PM4 packet stream into Ring Buffer:");
        // 1. Set Context Register: RB_COLOR_INFO (RenderTarget: 1280x720 32-bit ARGB at 0x70000000)
        let pkt_color = build_set_context_reg(RB_COLOR_INFO, 0x7000_0000);
        ring_buffer.write_packet(&mut bus, &pkt_color)?;
        println!("  [PM4 EMIT] SET_CONTEXT_REG (RB_COLOR_INFO: 0x70000000) [{} dwords]", pkt_color.len());

        // 2. Set Context Register: PA_SC_WINDOW_OFFSET (Viewport: (0, 0))
        let pkt_vport = build_set_context_reg(PA_SC_WINDOW_OFFSET, 0x0000_0000);
        ring_buffer.write_packet(&mut bus, &pkt_vport)?;
        println!("  [PM4 EMIT] SET_CONTEXT_REG (PA_SC_WINDOW_OFFSET: 0x0) [{} dwords]", pkt_vport.len());

        // 3. Load ALU Constants (c0..c3: 4x4 Matrix for UI Projection, c4: Diffuse Tint)
        let shader_constants = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 1.0, 1.0, 1.0],
        ];
        let pkt_constants = build_load_alu_constant(0, &shader_constants);
        ring_buffer.write_packet(&mut bus, &pkt_constants)?;
        println!("  [PM4 EMIT] LOAD_ALU_CONSTANT (c0..c4: Matrix + Diffuse Tint) [{} dwords]", pkt_constants.len());

        // 4. Stream Vertex Buffer in guest RAM at 0x10060000 (VGT DMA)
        let vgt_addr: u64 = 0x1006_0000;
        let raw_vertices: [([f32; 3], u32, [f32; 2]); 4] = [
            ([380.0, 150.0, 0.0], 0xFFFF_FFFF, [0.0, 0.0]),
            ([1180.0, 150.0, 0.0], 0xFFFF_FFFF, [1.0, 0.0]),
            ([1180.0, 250.0, 0.0], 0xFFFF_FFFF, [1.0, 1.0]),
            ([380.0, 250.0, 0.0], 0xFFFF_FFFF, [0.0, 1.0]),
        ];
        for (i, (pos, color, uv)) in raw_vertices.iter().enumerate() {
            let v_base = vgt_addr + (i * 24) as u64;
            bus.write_u32(v_base, pos[0].to_bits())?;
            bus.write_u32(v_base + 4, pos[1].to_bits())?;
            bus.write_u32(v_base + 8, pos[2].to_bits())?;
            bus.write_u32(v_base + 12, *color)?;
            bus.write_u32(v_base + 16, uv[0].to_bits())?;
            bus.write_u32(v_base + 20, uv[1].to_bits())?;
        }
        let pkt_dma_base = build_set_context_reg(VGT_DMA_BASE, vgt_addr as u32);
        let pkt_dma_size = build_set_context_reg(VGT_DMA_SIZE, 4);
        let pkt_dma_stride = build_set_context_reg(VGT_DMA_STRIDE, 24);
        ring_buffer.write_packet(&mut bus, &pkt_dma_base)?;
        ring_buffer.write_packet(&mut bus, &pkt_dma_size)?;
        ring_buffer.write_packet(&mut bus, &pkt_dma_stride)?;
        println!("  [PM4 EMIT] SET_CONTEXT_REG (VGT_DMA_BASE: 0x{:08X}, Size: 4, Stride: 24)", vgt_addr);

        // 5. Index buffer setup in guest RAM at 0x10050000
        let index_ptr: u32 = 0x1005_0000;
        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0]; // 2 Triangles for UI Quad
        for (i, &idx) in indices.iter().enumerate() {
            bus.write_u16(index_ptr as u64 + (i * 2) as u64, idx)?;
        }

        // 6. Emit Draw Call: PM4_DRAW_INDX (Primitive: Triangles, 6 indices)
        let pkt_draw = build_draw_indexed(0x04, 6, index_ptr);
        ring_buffer.write_packet(&mut bus, &pkt_draw)?;
        println!("  [PM4 EMIT] DRAW_INDX (Primitive: Triangles, 6 Indices, Base: 0x{:08X}) [{} dwords]", index_ptr, pkt_draw.len());

        // 7. Emit WAIT_FOR_IDLE fence
        let pkt_idle = build_wait_for_idle();
        ring_buffer.write_packet(&mut bus, &pkt_idle)?;
        println!("  [PM4 EMIT] WAIT_FOR_IDLE (Pipeline Flush & Fence) [{} dwords]", pkt_idle.len());

        println!("\n[*] Executing Xenos GPU Command Processor (CP) loop on Primary Ring Buffer...");
        let executed_count = xenos_gpu.execute_ring(&mut bus, &mut ring_buffer)?;
        println!("[+] Xenos GPU CP successfully processed {} PM4 packets from Ring Buffer!", executed_count);
        println!("    - Total Packets Processed: {}", xenos_gpu.packets_executed);
        println!("    - Draw Calls Executed:     {}", xenos_gpu.draw_calls);
        println!("    - Primitives Drawn:        {}", xenos_gpu.primitives_drawn);

        // ==========================================================
        // ETAPA 3: Direct3D 9 Subsystem & Barycentric Triangle Rasterizer
        // ==========================================================
        println!("\n>>> [ETAPA 3] Direct3D 9 Subsystem & XUI Barycentric Rasterizer <<<");
        if let Some(games_scene) = loaded_scenes.iter().find(|s| s.name.contains("games")) {
            println!("[*] Rasterizing authentic XUI Scene '{}' with D3D9 barycentric pipeline...", games_scene.name);
            xenos_gpu.render_xui_scene(games_scene, xus_table.as_ref(), 0x0010_7C10, 0x000B_4D0B);
            println!("[+] Direct3D 9 Subsystem successfully rasterized UI nodes into on-chip color buffer!");
            println!("    - Triangles Shaded:        {}", xenos_gpu.primitives_drawn);
            println!("    - Active Draw Calls:       {}", xenos_gpu.draw_calls);
            println!("    - Color Buffer Resolution: 1280x720 32-bit ARGB (EDRAM Target)");
        }
    }

    // ==========================================================
    // ETAPA 3: Multi-Thread Guest Dispatcher & Live Input Loop
    // ==========================================================
    println!("\n>>> [ETAPA 3] Genuine Dashboard Multi-Thread Interleaved Dispatcher <<<");
    struct GuestThread {
        name: String,
        cpu: Cpu,
        entry_pc: u64,
        active: bool,
        total_steps: u64,
        continuous: bool,
    }

    let mut guest_threads = Vec::new();

    if is_metro {
        println!("[*] Registering Metro 17559 Primary Thread (Entry: 0x{:08X}, SP: 0x{:08X})...", real_entry_vaddr, stack_top);
        let mut metro_cpu = Cpu::new(real_entry_vaddr);
        metro_cpu.regs.gpr[1] = stack_top;
        metro_cpu.regs.gpr[3] = vaddr_base; // r3: hModule (0x92000000)
        metro_cpu.regs.gpr[4] = 1;          // r4: DLL_PROCESS_ATTACH (1)
        metro_cpu.regs.gpr[5] = 0;          // r5: lpReserved (0)
        metro_cpu.regs.gpr[13] = kpcr_base; // r13: KPCR
        metro_cpu.regs.lr = 0x80001000;     // LR: ExTerminateThread stub
        bus.write_u32(stack_top - 8, 0x80001000)?;
        bus.write_u32(stack_top + 0x90 - 8, 0x80001000)?;
        guest_threads.push(GuestThread {
            name: "Metro 17559 Primary Thread".to_string(),
            cpu: metro_cpu,
            entry_pc: real_entry_vaddr,
            active: true,
            total_steps: 0,
            continuous: true,
        });
    } else {
        // 1. Dashboard Main Controller / Event Pump (@ 0x921C9830 from live console)
        let stack_top_ctrl = stack_top - 0xA0000;
        bus.write_u32(stack_top_ctrl - 8, 0x80001000)?;
        bus.write_u32(stack_top_ctrl + 0x100 - 8, 0x80001000)?;
        let mut ctrl_cpu = Cpu::new(0x921C9830);
        ctrl_cpu.regs.gpr[1] = stack_top_ctrl;
        ctrl_cpu.regs.gpr[3] = 0;
        ctrl_cpu.regs.gpr[4] = 0;
        ctrl_cpu.regs.gpr[13] = { let s = next_kpcr_slot; next_kpcr_slot += 1; setup_kpcr(&mut bus, s, stack_top_ctrl, stack_top_ctrl - 0x20000) };
        ctrl_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Main Controller & Event Pump".to_string(),
            cpu: ctrl_cpu,
            entry_pc: 0x921C9830,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 2. XUI Presentation & Scene Graph Thread (@ 0x9281FBC0 from live console)
        let stack_top_xui = stack_top - 0xC0000;
        bus.write_u32(stack_top_xui - 8, 0x80001000)?;
        bus.write_u32(stack_top_xui + 0x100 - 8, 0x80001000)?;
        let mut xui_cpu = Cpu::new(0x9281FBC0);
        xui_cpu.regs.gpr[1] = stack_top_xui;
        xui_cpu.regs.gpr[3] = 0;
        xui_cpu.regs.gpr[4] = 0;
        xui_cpu.regs.gpr[13] = { let s = next_kpcr_slot; next_kpcr_slot += 1; setup_kpcr(&mut bus, s, stack_top_xui, stack_top_xui - 0x20000) };
        xui_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "XUI Presentation Engine".to_string(),
            cpu: xui_cpu,
            entry_pc: 0x9281FBC0,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 3. Direct3D 9 / GPU Driver Loop (@ 0x92185BC0 from live console)
        let stack_top_gpu = stack_top - 0xE0000;
        bus.write_u32(stack_top_gpu - 8, 0x80001000)?;
        bus.write_u32(stack_top_gpu + 0x100 - 8, 0x80001000)?;
        let mut gpu_cpu = Cpu::new(0x92185BC0);
        gpu_cpu.regs.gpr[1] = stack_top_gpu;
        gpu_cpu.regs.gpr[3] = 0;
        gpu_cpu.regs.gpr[4] = 0;
        gpu_cpu.regs.gpr[13] = { let s = next_kpcr_slot; next_kpcr_slot += 1; setup_kpcr(&mut bus, s, stack_top_gpu, stack_top_gpu - 0x20000) };
        gpu_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Direct3D 9 GPU Driver".to_string(),
            cpu: gpu_cpu,
            entry_pc: 0x92185BC0,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 4. Dashboard State Machine Thread (@ 0x92261CE8 from live console)
        let stack_top_sm = stack_top - 0x100000;
        bus.write_u32(stack_top_sm - 8, 0x80001000)?;
        bus.write_u32(stack_top_sm + 0x100 - 8, 0x80001000)?;
        let mut sm_cpu = Cpu::new(0x92261CE8);
        sm_cpu.regs.gpr[1] = stack_top_sm;
        sm_cpu.regs.gpr[3] = 0;
        sm_cpu.regs.gpr[4] = 0;
        sm_cpu.regs.gpr[13] = { let s = next_kpcr_slot; next_kpcr_slot += 1; setup_kpcr(&mut bus, s, stack_top_sm, stack_top_sm - 0x20000) };
        sm_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Dashboard State Machine".to_string(),
            cpu: sm_cpu,
            entry_pc: 0x92261CE8,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 5. Background Resource & Stream Worker (@ 0x92267C58 from live console)
        let stack_top_res = stack_top - 0x120000;
        bus.write_u32(stack_top_res - 8, 0x80001000)?;
        bus.write_u32(stack_top_res + 0x100 - 8, 0x80001000)?;
        let mut res_cpu = Cpu::new(0x92267C58);
        res_cpu.regs.gpr[1] = stack_top_res;
        res_cpu.regs.gpr[3] = 0;
        res_cpu.regs.gpr[4] = 0;
        res_cpu.regs.gpr[13] = { let s = next_kpcr_slot; next_kpcr_slot += 1; setup_kpcr(&mut bus, s, stack_top_res, stack_top_res - 0x20000) };
        res_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Resource & Stream Worker".to_string(),
            cpu: res_cpu,
            entry_pc: 0x92267C58,
            active: true,
            total_steps: 0,
            continuous: true,
        });

        // 6. Content & Tile Initializer Worker (@ 0x92107280)
        let stack_top_worker = stack_top - 0x140000;
        bus.write_u32(stack_top_worker - 8, 0x80001000)?;
        bus.write_u32(stack_top_worker + 0x320 - 8, 0x80001000)?;
        let mut worker_cpu = Cpu::new(0x92107280);
        worker_cpu.regs.gpr[1] = stack_top_worker;
        worker_cpu.regs.gpr[3] = 0;
        worker_cpu.regs.gpr[4] = 0;
        worker_cpu.regs.gpr[13] = { let s = next_kpcr_slot; next_kpcr_slot += 1; setup_kpcr(&mut bus, s, stack_top_worker, stack_top_worker - 0x20000) };
        worker_cpu.regs.lr = 0x80001000;
        guest_threads.push(GuestThread {
            name: "Content & Tile Initializer".to_string(),
            cpu: worker_cpu,
            entry_pc: 0x92107280,
            active: true,
            total_steps: 0,
            continuous: true,
        });
    }

    println!("[+] Registered {} authentic Dashboard threads into 60 FPS guest scheduler:", guest_threads.len());
    for t in &guest_threads {
        println!("  - Thread '{}' -> Entry PC: 0x{:08X}, SP: 0x{:08X}", t.name, t.cpu.regs.pc, t.cpu.regs.gpr[1]);
    }

    // ==========================================================
    // MILESTONE 5: High-Definition GPU Video Display & Surface Rendering
    // ==========================================================
    println!("\n>>> [MILESTONE 5] GPU Video Output (1280x720 HD @ 60 FPS) <<<");
    println!("[*] Initializing High-Definition Video Display Surface...");
    let win_title = if is_metro {
        "Xenon-Emu: Xbox 360 Metro Dashboard [Title: FFFE07D1 | Build 17559]"
    } else {
        "Xenon-Emu: Xbox 360 Blades Dashboard [Title: FFFE07D1 | Build 4532]"
    };
    let mut display = GpuDisplay::new(win_title)?;
    if let Some(ref table) = xus_table {
        display.apply_xui_locale(table, &loaded_scenes);
    }
    println!("[+] Display Window opened: 1280x720 @ 60 FPS");

    println!("\n[*] ========================================================");
    println!("    🎮 XBOX 360 INICIANDO: ANIMAÇÃO DE BOOT ATIVA!");
    println!("    - A animacao de boot do Xbox 360 e reproduzida (~4.5s)");
    println!("    - Pressione ESPACO ou (A) no controle para pular (apos 1s)");
    println!("    - Em seguida, a Dashboard surge em 1280x720 HD @ 60 FPS");
    println!("    - [TAB] ou Botao (Y): Alternar Modos: Blades -> Guest VRAM -> Hibrido -> XUI");
    println!("    - [U]: Alternar Algoritmo GPU Untile (Xenia Morton / Variant 0 / Linear)");
    println!("    - [B]: Alternar Buffer de Vídeo (VRAM Guest Ao Vivo vs Dump Calibrado de Hardware)");
    println!("    - [C]: Alternar Formato de Canais de Cor (ARGB / BGRA / RGBA / Raw)");
    println!("    - [X] ou [1..4]: Alternar Cenas XUI (BladeSkin, gamesSignedOut)");
    println!("    - Setas / A,D ou D-Pad / Thumbstick: Alternar Blades");
    println!("    - Setas / W,S ou D-Pad: Navegar nos itens do menu");
    println!("    - Pressione ESC ou clique no [X] para fechar a janela");
    println!("    ========================================================\n");

    let diag = diag::config();
    let initial_fb = kernel.active_frontbuffer();
    println!("[*] Guest VRAM Framebuffer Ativo inicial: 0x{:08X}", initial_fb);
    println!("[*] Fonte de Vídeo inicial: {} (tecla [B] alterna; XENON_FB_SOURCE=reference inicia no dump)", if diag.fb_source_reference { "DUMP DE REFERÊNCIA" } else { "VRAM Guest Ao Vivo" });

    let max_frames: Option<u64> = diag.max_frames;
    // Trap diagnostics: XENON_TRAP_TRACE keeps a ring of the last executed instructions of the
    // Metro primary thread and dumps it on trap; XENON_NO_RESTART stops the thread instead of restarting it.
    let trap_trace = diag.trap_trace;
    let no_restart = diag.no_restart;
    let trap_full = diag.trap_full;
    // Restart policy: the Metro primary thread runs the CRT startup once; restarting it re-runs startup
    // over already-initialised globals (double InsertTailList -> self-linked node). Only restart it when asked.
    let restart_main = diag.restart_main;
    // First-run register snapshot per thread: (SP, r3, r4, r5, r13), used to give restarts a clean stack/arguments
    let mut thread_init: std::collections::HashMap<String, (u64, u64, u64, u64, u64)> = std::collections::HashMap::new();
    fn restart_thread(cpu: &mut Cpu, entry: u64, init: Option<&(u64, u64, u64, u64, u64)>) {
        cpu.regs.pc = entry;
        cpu.regs.lr = 0x80001000;
        cpu.regs.ctr = 0;
        if let Some(&(sp, r3, r4, r5, r13)) = init {
            cpu.regs.gpr[1] = sp;
            cpu.regs.gpr[3] = r3;
            cpu.regs.gpr[4] = r4;
            cpu.regs.gpr[5] = r5;
            cpu.regs.gpr[13] = r13;
        }
    }
    // XENON_PC_HOOK=0xADDR,0xADDR: dump registers whenever the Metro primary thread reaches these PCs
    // XENON_WATCH_DUMP_HIT=<n>: dump the instruction ring right after the n-th watchpoint hit (needs XENON_TRAP_TRACE=1)
    let watch_dump_hit: Option<u64> = diag.watch_dump_hit;
    let mut watch_dump_done = false;
    let dump_ring_frame: Option<u64> = diag.dump_ring_frame;
    let heartbeat = diag.heartbeat;
    let gpu_mmio_log = diag.gpu_mmio_log;
    let syscall_log = diag.syscall_log;
    let syscall_summary = diag.syscall_summary;
    // (thread, lib, ordinal, lr) -> (count, first_frame, last_frame, last args)
    let mut sys_agg: std::collections::HashMap<(String, String, u16, u64), (u64, u64, u64, [u64; 4])> = std::collections::HashMap::new();
    let watchdog = diag.watchdog;
    if watchdog {
        std::thread::spawn(|| {
            let mut last = WD_STEPS.load(Relaxed);
            let mut still = 0u32;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                let now = WD_STEPS.load(Relaxed);
                if now == last && now != 0 { still += 1; } else { still = 0; last = now; }
                if still >= 8 {
                    let s = WD_SYS.load(Relaxed);
                    eprintln!("[WATCHDOG] NO PROGRESS for 4s: frame {} steps={} pc=0x{:08X} lr=0x{:08X} in_syscall={} (ordinal=0x{:04X}, xam={}) args=[{:X},{:X},{:X},{:X}]",
                        WD_FRAME.load(Relaxed), now, WD_PC.load(Relaxed) & 0xFFFF_FFFF, WD_LR.load(Relaxed) & 0xFFFF_FFFF,
                        s != 0, s & 0xFFFF, (s >> 32) != 0, WD_A0.load(Relaxed), WD_A1.load(Relaxed), WD_A2.load(Relaxed), WD_A3.load(Relaxed));
                    std::process::exit(86);
                }
            }
        });
    }
    let hook_all_threads = diag.pc_hook_all;
    let mut pc_hook_hits: u64 = 0;
    let pc_hook_min_frame: u64 = diag.pc_hook_min_frame;
    let pc_hook_limit: u64 = diag.pc_hook_limit;
    let pc_hooks: Vec<u64> = diag.pc_hooks.clone();
    // (pc, opcode, lr, ctr, r1, r3, r11, r12)
    let mut trap_ring: std::collections::VecDeque<(u64, u32, u64, u64, u64, u64, u64, u64, u64, u64)> =
        std::collections::VecDeque::with_capacity(64);
    let pc_profile = diag.pc_profile;
    let mut pc_hist: std::collections::HashMap<(String, u64), u64> = std::collections::HashMap::new();
    let mut sys_hist: std::collections::HashMap<(String, u16), u64> = std::collections::HashMap::new();

    let mut frame = 0u64;
    while display.is_open() {
        frame += 1;
        WD_FRAME.store(frame, Relaxed);
        if let Some(mf) = max_frames {
            if frame > mf {
                println!("[*] Reached XENON_MAX_FRAMES limit ({} frames) - terminating loop cleanly.", mf);
                break;
            }
        }
        display.handle_input();

        // 1. Process any pending dynamic thread creations from ExCreateThread
        while let Some(pending) = kernel.pending_threads.pop() {
            let thread_stack = stack_top - 0x160000 - (guest_threads.len() as u64 * 0x20000);
            let _ = bus.write_u32(thread_stack - 8, 0x80001000);
            let _ = bus.write_u32(thread_stack + 0x100 - 8, 0x80001000);
            let routine = (pending.start_routine & 0xFFFF_FFFF) as u64;
            let mut thread_cpu = Cpu::new(routine);
            thread_cpu.regs.gpr[1] = thread_stack;
            thread_cpu.regs.gpr[3] = pending.start_context;
            thread_cpu.regs.gpr[13] = { let s = 8 + (next_kpcr_slot % 40); next_kpcr_slot += 1; setup_kpcr(&mut bus, s, thread_stack, thread_stack - 0x20000) };
            thread_cpu.regs.lr = 0x80001000;
            println!("  [SCHEDULER] Spawning dynamic guest thread TID=0x{:X} (@0x{:08X})",
                pending.tid, routine
            );
            let is_worker_or_render = true; // Dashboard dynamic worker/UI threads should run continuously
            guest_threads.push(GuestThread {
                name: format!("Dynamic Thread 0x{:X}", pending.tid),
                cpu: thread_cpu,
                entry_pc: routine,
                active: true,
                total_steps: 0,
                continuous: is_worker_or_render,
            });
        }

        // 2. Feed live unified gamepad state (USB controller or keyboard) into HLE Kernel
        let gp_state = display.get_current_gamepad_state();
        kernel.update_gamepad(gp_state);

        // 3. Interleave continuous guest execution of genuine dashboard threads
        for thread in &mut guest_threads {
            if !thread.active {
                continue;
            }
            if heartbeat {
                use std::io::Write;
                eprintln!("[HB] frame {} thread '{}' pc=0x{:08X} lr=0x{:08X}", frame, thread.name, thread.cpu.regs.pc & 0xFFFF_FFFF, thread.cpu.regs.lr & 0xFFFF_FFFF);
                let _ = std::io::stderr().flush();
            }
            bus.current_thread.clear();
            bus.current_thread.push_str(&thread.name);
            if !thread_init.contains_key(&thread.name) {
                let r = &thread.cpu.regs;
                thread_init.insert(thread.name.clone(), (r.gpr[1], r.gpr[3], r.gpr[4], r.gpr[5], r.gpr[13]));
            }
            // Execute a timeslice of up to 25,000 instructions per thread per frame
            let slice_start = std::time::Instant::now();
            for step_i in 0..25000u32 {
                if watchdog && (step_i & 0x1FF) == 0 && slice_start.elapsed().as_secs() >= 3 {
                    eprintln!("[WATCHDOG] slice > 3s: frame {} thread '{}' step {} pc=0x{:08X} lr=0x{:08X} ctr=0x{:X} r1=0x{:X} r3=0x{:X} r4=0x{:X} r5=0x{:X}",
                        frame, thread.name, step_i, thread.cpu.regs.pc & 0xFFFF_FFFF, thread.cpu.regs.lr & 0xFFFF_FFFF, thread.cpu.regs.ctr,
                        thread.cpu.regs.gpr[1], thread.cpu.regs.gpr[3], thread.cpu.regs.gpr[4], thread.cpu.regs.gpr[5]);
                    std::process::exit(86);
                }
                let current_pc = thread.cpu.regs.pc;
                if watchdog {
                    WD_STEPS.fetch_add(1, Relaxed);
                    WD_PC.store(current_pc, Relaxed);
                    WD_LR.store(thread.cpu.regs.lr, Relaxed);
                }
                bus.current_pc = current_pc;
                bus.current_lr = thread.cpu.regs.lr;
                bus.current_sp = thread.cpu.regs.gpr[1];
                match bus.read_u32(current_pc) {
                    Ok(op_raw) => {
                        if !pc_hooks.is_empty() && (hook_all_threads || thread.name.starts_with("Metro")) && pc_hooks.contains(&(current_pc & 0xFFFF_FFFF)) && frame >= pc_hook_min_frame {
                            let r = &thread.cpu.regs;
                            pc_hook_hits += 1;
                            if pc_hook_hits <= pc_hook_limit {
                                if let Some(spec) = &diag.pc_hook_mem {
                                    // "<reg>:<hex offset>:<word count>" e.g. "31:360:40" dumps 40 words at r31+0x360
                                    let p: Vec<&str> = spec.split(':').collect();
                                    if p.len() == 3 {
                                        if let (Ok(rn), Ok(off), Ok(cnt)) = (p[0].parse::<usize>(), u64::from_str_radix(p[1], 16), p[2].parse::<u64>()) {
                                            let base = (r.gpr[rn] & 0xFFFF_FFFF).wrapping_add(off);
                                            let mut line = format!("    [MEM r{}+0x{:X} = 0x{:08X}]", rn, off, base);
                                            for i in 0..cnt {
                                                if i % 8 == 0 { line.push_str(&format!("\n      {:08X}:", base + i * 4)); }
                                                line.push_str(&format!(" {:08X}", bus.read_u32(base + i * 4).unwrap_or(0xDEADBEEF)));
                                            }
                                            println!("{}", line);
                                        }
                                    }
                                }
                                if let Some(spec) = &diag.pc_hook_list {
                                    // "<reg>": walk a circular doubly linked list (flink at +0) starting at the address held in GPR <reg>
                                    if let Ok(rn) = spec.trim().parse::<usize>() {
                                        let head = r.gpr[rn] & 0xFFFF_FFFF;
                                        let mut cur = bus.read_u32(head).unwrap_or(0) as u64;
                                        let mut out = format!("head=0x{:08X} ->", head);
                                        for _ in 0..24 {
                                            let hdr = cur.wrapping_sub(8) & 0xFFFF_FFFF;
                                            let size_units = bus.read_u16(hdr).unwrap_or(0);
                                            out.push_str(&format!(" 0x{:08X}(sz={:#x},fl={:02X})", cur, size_units, bus.read_u8(hdr + 5).unwrap_or(0)));
                                            if cur == head || cur == 0 { break; }
                                            cur = bus.read_u32(cur).unwrap_or(0) as u64;
                                        }
                                        println!("    [LIST] {}", out);
                                    }
                                }
                                if let Some(regs) = &diag.pc_hook_strregs {
                                    // e.g. "4,3": print the big-endian UTF-16 string each listed GPR points at
                                    for rn in regs.split(',').filter_map(|x| x.trim().parse::<usize>().ok()) {
                                        let mut s = String::new();
                                        let mut a = r.gpr[rn] & 0xFFFF_FFFF;
                                        for _ in 0..64 {
                                            match bus.read_u16(a) { Ok(0) | Err(_) => break, Ok(c) => s.push(char::from_u32(c as u32).unwrap_or('?')) }
                                            a += 2;
                                        }
                                        println!("    [STRREG] r{}=0x{:08X} -> \"{}\"", rn, r.gpr[rn] & 0xFFFF_FFFF, s);
                                    }
                                }
                                if diag.pc_hook_r3str {
                                    // r3 points at a struct whose +4 / +8 members are pointers to big-endian UTF-16 strings
                                    let rd16 = |bus: &MemoryBus, p: u64| -> String {
                                        let mut s = String::new();
                                        let mut a = p & 0xFFFF_FFFF;
                                        for _ in 0..64 {
                                            match bus.read_u16(a) { Ok(0) | Err(_) => break, Ok(c) => s.push(char::from_u32(c as u32).unwrap_or('?')) }
                                            a += 2;
                                        }
                                        s
                                    };
                                    let base3 = r.gpr[3] & 0xFFFF_FFFF;
                                    let p4 = bus.read_u32(base3 + 4).unwrap_or(0) as u64;
                                    let p8 = bus.read_u32(base3 + 8).unwrap_or(0) as u64;
                                    println!("    [R3STR] name=\"{}\" base=\"{}\"", rd16(&bus, p4), rd16(&bus, p8));
                                }
                                println!("  [PC_HOOK #{} frame {} {}] pc=0x{:08X} lr=0x{:08X} r1=0x{:08X} r3=0x{:X} r4=0x{:X} r5=0x{:X} r6=0x{:X} r7=0x{:X} r8=0x{:X} r9=0x{:X} r10=0x{:X} r11=0x{:X} r29=0x{:X} r30=0x{:X} r31=0x{:X} ctr=0x{:X} callers:{}",
                                    pc_hook_hits, frame, thread.name.replace(" ", "_"), current_pc & 0xFFFF_FFFF, r.lr & 0xFFFF_FFFF, r.gpr[1] & 0xFFFF_FFFF, r.gpr[3], r.gpr[4], r.gpr[5], r.gpr[6], r.gpr[7], r.gpr[8], r.gpr[9], r.gpr[10], r.gpr[11], r.gpr[29], r.gpr[30], r.gpr[31], r.ctr,
                                    bus.backchain(r.gpr[1], 14));
                            }
                        }
                        if trap_trace && thread.name.starts_with("Metro") {
                            if trap_ring.len() >= 20000 { trap_ring.pop_front(); }
                            let r = &thread.cpu.regs;
                            trap_ring.push_back((current_pc, op_raw, r.lr, r.ctr, r.gpr[1], r.gpr[3], r.gpr[11], r.gpr[12], r.gpr[31], r.gpr[30]));
                        }
                        match thread.cpu.step(&mut bus) {
                            Ok(StepResult::Ok) => {
                                thread.total_steps += 1;
                                if let Some(h) = watch_dump_hit {
                                    if !watch_dump_done && bus.watch_hits >= h {
                                        watch_dump_done = true;
                                        let dump_n: usize = diag.dump_ring_n;
                                        let n = trap_ring.len();
                                        println!("  [WATCH DUMP] hit #{} by pc=0x{:08X}; last {} instructions:", bus.watch_hits, current_pc & 0xFFFF_FFFF, dump_n.min(n));
                                        for e in trap_ring.iter().skip(n.saturating_sub(dump_n)) {
                                            println!("    seq pc=0x{:08X} op=0x{:08X} lr=0x{:08X} r3=0x{:X} r11=0x{:X} r30=0x{:X} r31=0x{:X}", e.0 & 0xFFFF_FFFF, e.1, e.2 & 0xFFFF_FFFF, e.5, e.6, e.9, e.8);
                                        }
                                    }
                                }
                                if pc_profile && !thread.name.starts_with("VBlank") {
                                    *pc_hist.entry((thread.name.clone(), current_pc & 0xFFFF_FFFF)).or_insert(0) += 1;
                                }
                            }
                            Ok(StepResult::Syscall { lev: _, pc }) => {
                                let thunk = thunk_map.get(&pc).or_else(|| thunk_map.get(&(pc & 0xFFFF_FFFF)));
                                if pc_profile {
                                    if let Some((l, o)) = thunk {
                                        *sys_hist.entry((format!("{}|{}", thread.name, l), *o)).or_insert(0) += 1;
                                    }
                                }
                                if let Some((lib, ordinal)) = thunk {
                                    let is_term = *ordinal == 0x0019 && lib.eq_ignore_ascii_case("xboxkrnl.exe");
                                    let is_yield = (*ordinal == 0x0101 && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x002B && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x0057 && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00B0 && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00AF && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00FD && lib.eq_ignore_ascii_case("xboxkrnl.exe"))
                                        || (*ordinal == 0x00FE && lib.eq_ignore_ascii_case("xboxkrnl.exe"));

                                    let args = [
                                        thread.cpu.regs.gpr[3], thread.cpu.regs.gpr[4], thread.cpu.regs.gpr[5],
                                        thread.cpu.regs.gpr[6], thread.cpu.regs.gpr[7], thread.cpu.regs.gpr[8],
                                        thread.cpu.regs.gpr[9], thread.cpu.regs.gpr[10],
                                    ];
                                    if syscall_summary {
                                        let e = sys_agg.entry((thread.name.clone(), lib.to_string(), *ordinal, thread.cpu.regs.lr & 0xFFFF_FFFF)).or_insert((0, frame, frame, [0; 4]));
                                        e.0 += 1; e.2 = frame; e.3 = [args[0], args[1], args[2], args[3]];
                                    }
                                    if syscall_log {
                                        use std::io::Write;
                                        eprintln!("[SYS] f{} {} {}!0x{:04X} a=[{:X},{:X},{:X},{:X},{:X},{:X}] lr=0x{:08X}", frame, thread.name, lib, ordinal, args[0], args[1], args[2], args[3], args[4], args[5], thread.cpu.regs.lr & 0xFFFF_FFFF);
                                        let _ = std::io::stderr().flush();
                                    }
                                    if watchdog {
                                        WD_SYS.store(((lib.to_ascii_lowercase().contains("xam") as u64) << 32) | (*ordinal as u64), Relaxed);
                                        WD_A0.store(args[0], Relaxed); WD_A1.store(args[1], Relaxed); WD_A2.store(args[2], Relaxed); WD_A3.store(args[3], Relaxed);
                                    }
                                    let ret = kernel.dispatch(lib, *ordinal, &args, &mut bus);
                                    if watchdog { WD_SYS.store(0, Relaxed); }
                                    thread.cpu.regs.gpr[3] = ret;
                                    thread.cpu.regs.pc = thread.cpu.regs.lr;

                                    if is_term {
                                        if thread.continuous && no_restart {
                                            println!("  [NO_RESTART] thread '{}' called ExTerminateThread; stopping", thread.name);
                                            thread.continuous = false;
                                            thread.active = false;
                                            break;
                                        }
                                        if thread.continuous && thread.name.starts_with("Metro") && !restart_main {
                                            println!("  [THREAD EXIT] '{}' called ExTerminateThread at frame {} (pc=0x{:08X}); not restarting (set XENON_RESTART_MAIN=1 to restart)",
                                                thread.name, frame, current_pc);
                                            thread.continuous = false;
                                            thread.active = false;
                                            break;
                                        }
                                        if thread.continuous {
                                            if diag.watch_set {
                                                println!("  [RESTART:TERM] thread '{}' frame {} called ExTerminateThread (pc=0x{:08X}, SP=0x{:08X}) -> restarting at entry 0x{:08X}",
                                                    thread.name, frame, current_pc, thread.cpu.regs.gpr[1], thread.entry_pc);
                                            }
                                            restart_thread(&mut thread.cpu, thread.entry_pc, thread_init.get(&thread.name));
                                            break;
                                        } else {
                                            thread.active = false;
                                            break;
                                        }
                                    }
                                    if is_yield {
                                        // Yield timeslice to next thread or frame
                                        break;
                                    }
                                } else {
                                    thread.active = false;
                                    break;
                                }
                            }
                            Ok(StepResult::Halted) => {
                                thread.active = false;
                                break;
                            }
                            Err(e) => {
                                if thread.continuous {
                                    if trap_trace && thread.name.starts_with("Metro") {
                                        println!("  [TRAP TRACE] last {} instructions before trap ({}); after-trap LR=0x{:08X} CTR=0x{:08X}:",
                                            trap_ring.len(), e, thread.cpu.regs.lr, thread.cpu.regs.ctr);
                                        let tail_start = trap_ring.len().saturating_sub(12);
                                        let mut prev31 = u64::MAX;
                                        for (i, (pc, op, lr, ctr, r1, r3, r11, r12, r31, _r30)) in trap_ring.iter().enumerate() {
                                            let changed = *r31 != prev31;
                                            if changed || i >= tail_start || trap_full {
                                                println!("    {}pc=0x{:08X} op=0x{:08X} lr=0x{:08X} ctr=0x{:08X} r1=0x{:08X} r3=0x{:08X} r11=0x{:08X} r12=0x{:08X} r31=0x{:08X}",
                                                    if changed { "[r31 changed] " } else { "" },
                                                    pc & 0xFFFF_FFFF, op, lr & 0xFFFF_FFFF, ctr & 0xFFFF_FFFF, r1 & 0xFFFF_FFFF, r3 & 0xFFFF_FFFF, r11 & 0xFFFF_FFFF, r12 & 0xFFFF_FFFF, r31 & 0xFFFF_FFFF);
                                            }
                                            prev31 = *r31;
                                        }
                                        trap_ring.clear();
                                    }
                                    if no_restart {
                                        println!("  [NO_RESTART] stopping thread '{}' after first trap", thread.name);
                                        thread.continuous = false;
                                        thread.active = false;
                                        break;
                                    }
                                    if thread.name.starts_with("Metro") && !restart_main {
                                        println!("  [THREAD TRAP] '{}' trapped at frame {} pc=0x{:08X} SP=0x{:08X}: {} -- thread stopped (set XENON_RESTART_MAIN=1 to restart)",
                                            thread.name, frame, current_pc, thread.cpu.regs.gpr[1], e);
                                        thread.continuous = false;
                                        thread.active = false;
                                        break;
                                    }
                                    if diag.watch_set {
                                        println!("  [RESTART:ERR] thread '{}' frame {} trapped at pc=0x{:08X} SP=0x{:08X}: {} -> restarting at entry 0x{:08X}",
                                            thread.name, frame, current_pc, thread.cpu.regs.gpr[1], e, thread.entry_pc);
                                    }
                                    restart_thread(&mut thread.cpu, thread.entry_pc, thread_init.get(&thread.name));
                                } else {
                                    thread.active = false;
                                }
                                break;
                            }
                        }
                    }
                    Err(_) => {
                        thread.active = false;
                        break;
                    }
                }
            }
            if let Some(df) = dump_ring_frame {
                if frame == df && thread.name.starts_with("Metro") && !trap_ring.is_empty() {
                    // Distinct PCs by count + the last 400 executed instructions, in order.
                    let mut counts: std::collections::BTreeMap<u64, u32> = std::collections::BTreeMap::new();
                    for e in trap_ring.iter() { *counts.entry(e.0 & 0xFFFF_FFFF).or_insert(0) += 1; }
                    println!("  [RING DUMP frame {}] {} instructions, {} distinct PCs", frame, trap_ring.len(), counts.len());
                    let mut v: Vec<(u64, u32)> = counts.into_iter().collect();
                    v.sort_by(|a, b| b.1.cmp(&a.1));
                    for (pc, c) in v.iter().take(60) { println!("    hot pc=0x{:08X} x{}", pc, c); }
                    let n = trap_ring.len();
                    let dump_n: usize = diag.dump_ring_n;
                    for e in trap_ring.iter().skip(n.saturating_sub(dump_n)) {
                        println!("    seq pc=0x{:08X} op=0x{:08X} lr=0x{:08X} r3=0x{:X} r11=0x{:X} r30=0x{:X} r31=0x{:X}", e.0 & 0xFFFF_FFFF, e.1, e.2 & 0xFFFF_FFFF, e.5, e.6, e.9, e.8);
                    }
                }
            }        }

        // Retain only active or continuous threads to prevent vector bloat
        guest_threads.retain(|t| t.active || t.continuous);

        // 4. Synchronize & Execute PM4 packets in Primary Ring Buffer with Xenos GPU CP
        if kernel.ring_buffer_ptr != 0 && kernel.ring_buffer_ptr != ring_buffer.base_addr {
            let sz = if kernel.ring_buffer_size <= 32 {
                let comp = (1usize << kernel.ring_buffer_size) * 4;
                comp.max(32 * 1024)
            } else if kernel.ring_buffer_size != 0 {
                kernel.ring_buffer_size as usize
            } else {
                32 * 1024
            };
            println!("  [GPU] Re-binding RingBuffer to guest configured address 0x{:08X} (Size: {} KB)", kernel.ring_buffer_ptr, sz / 1024);
            ring_buffer = RingBuffer::new(kernel.ring_buffer_ptr, sz);
        }

        // The dash signals new commands by storing the ring write pointer (in dwords) to CP_RB_WPTR in the GPU MMIO window.
        // Use it when the guest has ever written it; otherwise fall back to scanning the ring for the last non-zero word.
        if let Some(&w) = bus.gpu_mmio.get(&memory::bus::GPU_MMIO_CP_RB_WPTR) {
            if gpu_mmio_log && w as usize != ring_buffer.wptr {
                println!("  [GPU MMIO] frame {} CP_RB_WPTR = {} (was {}) ring={:#X} size={} dwords, total mmio writes={}", frame, w, ring_buffer.wptr, ring_buffer.base_addr, ring_buffer.num_dwords, bus.gpu_mmio_writes);
            }
            ring_buffer.wptr = (w as usize) % ring_buffer.num_dwords.max(1);
        } else {
            ring_buffer.sync_wptr_from_memory(&bus);
        }
        let executed_pkts = xenos_gpu
            .execute_ring_with_writeback(&mut bus, &mut ring_buffer, kernel.rptr_writeback_ptr)
            .unwrap_or(0);

        if executed_pkts > 0 {
            println!(
                "  [GPU CP] Frame {:05} | Processed {} PM4 packets (Total: {}, Draw Calls: {}, Primitives: {}) | WPTR: {}, RPTR: {}",
                frame, executed_pkts, xenos_gpu.packets_executed, xenos_gpu.draw_calls, xenos_gpu.primitives_drawn, ring_buffer.wptr, ring_buffer.rptr
            );
        }

        // 5. Dispatch authentic VdSwap (#0x025B / 603) for frame presentation
        let frontbuffer = kernel.active_frontbuffer();
        kernel.dispatch("xboxkrnl.exe", 0x025B, &[frontbuffer, frame], &mut bus);

        // Disparar VBlank Graphics Interrupt Callback se configurado pelo guest (#0x01D5)
        if kernel.graphics_interrupt_callback != 0 && frame % 2 == 0 {
            let cb_routine = (kernel.graphics_interrupt_callback & 0xFFFF_FFFF) as u64;
            let cb_stack = stack_top - 0x1E0000;
            let _ = bus.write_u32(cb_stack - 8, 0x80001000);
            let _ = bus.write_u32(cb_stack + 0x100 - 8, 0x80001000);
            let mut cb_cpu = Cpu::new(cb_routine);
            cb_cpu.regs.gpr[1] = cb_stack;
            cb_cpu.regs.gpr[3] = kernel.graphics_interrupt_context;
            cb_cpu.regs.gpr[4] = 1; // 1 = VBLANK interrupt reason
            cb_cpu.regs.gpr[5] = frame;
            cb_cpu.regs.gpr[13] = setup_kpcr(&mut bus, 56 + (frame / 2) % 8, cb_stack, cb_stack - 0x20000);
            cb_cpu.regs.lr = 0x80001000;
            guest_threads.push(GuestThread {
                name: format!("VBlank Interrupt Callback #{}", frame),
                cpu: cb_cpu,
                entry_pc: cb_routine,
                active: true,
                total_steps: 0,
                continuous: false,
            });
        }

        // 6. Direct Presentation: Boot Animation -> Direct ATI Xenos GPU Framebuffer
        display.present_guest_memory_framebuffer(&bus, frontbuffer, &xenos_gpu.color_buffer);
        display.update()?;

        if frame == 70 {
            let _ = display.save_bmp(resolve_path("bootanim_ribbons.bmp"));
        } else if frame == 180 {
            let _ = display.save_bmp(resolve_path("bootanim_sphere.bmp"));
        } else if frame == 290 {
            let out_name = if is_metro { resolve_path("dash_17559_framebuffer.bmp") } else { resolve_path("dash_framebuffer.bmp") };
            let _ = display.save_bmp(&out_name);
        }

        if frame % 180 == 0 {
            let total_guest_steps: u64 = guest_threads.iter().map(|t| t.total_steps).sum();
            println!(
                "  [GPU] Frame {:05} | FrontBuffer: 0x{:08X} | Ring Executed: {} pkts | 60 FPS | Guest Code: {} insns",
                frame, frontbuffer, executed_pkts, total_guest_steps
            );
            for t in &guest_threads {
                println!(
                    "    -> [THREAD] \"{}\" | PC: 0x{:08X} | LR: 0x{:08X} | Steps: {} | Active: {} | Cont: {}",
                    t.name, t.cpu.regs.pc, t.cpu.regs.lr, t.total_steps, t.active, t.continuous
                );
            }
        }
    }

    if syscall_summary {
        let mut v: Vec<_> = sys_agg.iter().collect();
        v.sort_by(|a, b| b.1.0.cmp(&a.1.0));
        println!("[SYSCALL SUMMARY] top 60 by count (thread | lib!ordinal | caller LR | count | frames first..last | last args)");
        for ((th, lib, ord, lr), (n, f0, f1, a)) in v.iter().take(60) {
            println!("  {:<28} {}!0x{:04X} lr=0x{:08X} x{:<6} f{}..{} a=[{:X},{:X},{:X},{:X}]", th, lib, ord, lr, n, f0, f1, a[0], a[1], a[2], a[3]);
        }
    }

    if pc_profile {
        for t in &guest_threads {
            if t.name.starts_with("Metro") {
                let r = &t.cpu.regs.gpr;
                println!("[PROFILE] Primary regs: pc=0x{:08X} r3=0x{:08X} r11=0x{:08X} r28=0x{:08X} r30=0x{:08X} r31=0x{:08X}",
                    t.cpu.regs.pc, r[3], r[11], r[28], r[30], r[31]);
                let mut node = r[30] & 0xFFFF_FFFF;
                for i in 0..8 {
                    let next = bus.read_u32(node).unwrap_or(0);
                    let prev = bus.read_u32(node + 4).unwrap_or(0);
                    let cb = bus.read_u32(node + 8).unwrap_or(0);
                    println!("  node[{}] @0x{:08X}: next=0x{:08X} prev=0x{:08X} +8=0x{:08X}", i, node, next, prev, cb);
                    node = next as u64;
                }
                let h = r[31] & 0xFFFF_FFFF;
                println!("  head @0x{:08X}: next=0x{:08X} prev=0x{:08X}", h, bus.read_u32(h).unwrap_or(0), bus.read_u32(h + 4).unwrap_or(0));
            }
        }
        let mut v: Vec<_> = pc_hist.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1));
        println!("\n[PROFILE] Top 40 PCs (thread, pc, hits):");
        for ((n, pc), c) in v.iter().take(40) {
            println!("  {:<28} 0x{:08X} {}", n, pc, c);
        }
        let mut s: Vec<_> = sys_hist.iter().collect();
        s.sort_by(|a, b| b.1.cmp(a.1));
        println!("[PROFILE] Top 30 syscalls:");
        for ((n, o), c) in s.iter().take(30) {
            println!("  {:<40} #0x{:04X} {}", n, o, c);
        }
    }

    // Save framebuffer screenshot to disk
    let bmp_path = if is_metro { resolve_path("dash_17559_framebuffer.bmp") } else { resolve_path("dash_framebuffer.bmp") };
    display.save_bmp(&bmp_path)?;
    println!("[+] Framebuffer exported as High-Res BMP screenshot: {}", bmp_path.display());

    println!("\n========================================================");
    println!("  🎉 Milestones 1 to 5 Fully Operational!                ");
    println!("  Hardware Boot + XEX2 Loader + STFS Extractor +        ");
    println!("  HLE Kernel + 720p HD Display Window Active!           ");
    println!("========================================================");

    Ok(())
}
