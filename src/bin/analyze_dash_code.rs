use byteorder::{BigEndian, ByteOrder};
use std::fs;
use xenon_emu::loader::{PeImage, StfsPackage, XexFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Deep Code & Symbol Analysis of $flash_dash.xex     ");
    println!("========================================================");

    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let pe_data = dash_xex.decompress_image(&decrypted)?;
    let pe = PeImage::parse(&pe_data)?;

    let vaddr_base = dash_xex.load_address as u64;

    println!("[*] PE Image Size: {} bytes ({} MB)", pe_data.len(), pe_data.len() / (1024 * 1024));
    println!("[*] Number of sections: {}", pe.sections.len());
    for sec in &pe.sections {
        println!("    - Section {:<10} RVA: 0x{:08X} VSize: 0x{:08X} RawOffset: 0x{:08X} RawSize: 0x{:08X}",
            sec.name, sec.virtual_address, sec.virtual_size, sec.raw_data_ptr, sec.raw_data_size
        );
    }

    // Scan for all references to XamAppRegister, XamAppAttach, Xui, or thread creators
    // In PPC, 'bl' has opcode 18: [010010][LI 24 bits][AA][LK]
    // 0x48000001 to 0x4BFFFFFD

    // Let's scan for interesting ASCII and UTF-16 strings in .rdata and .data
    println!("\n[*] Scanning for interesting strings in $flash_dash.xex:");
    let mut found_strings = Vec::new();
    let mut i = 0;
    while i < pe_data.len().saturating_sub(6) {
        // ASCII check (at least 6 printable chars)
        if pe_data[i].is_ascii_graphic() {
            let start = i;
            while i < pe_data.len() && (pe_data[i].is_ascii_graphic() || pe_data[i] == b' ') {
                i += 1;
            }
            if i - start >= 6 && i < pe_data.len() && pe_data[i] == 0 {
                if let Ok(s) = std::str::from_utf8(&pe_data[start..i]) {
                    let lower = s.to_ascii_lowercase();
                    if lower.contains("xui") || lower.contains("dash") || lower.contains("blade")
                        || lower.contains("main") || lower.contains("init") || lower.contains("run")
                        || lower.contains("scene") || lower.contains("xbox") || lower.contains("create")
                    {
                        let rva = start as u32;
                        let vaddr = vaddr_base + rva as u64;
                        found_strings.push((vaddr, s.to_string()));
                    }
                }
            }
        }
        i += 1;
    }

    println!("[+] Found {} interesting strings.", found_strings.len());

    // Find cross references to key functions:
    // VdInitializeEngines thunk:
    // Let's find where VdInitializeEngines is in the thunk table
    // Find .text section
    let text_sec = pe.sections.iter().find(|s| s.name.eq_ignore_ascii_case(".text")).expect("No .text section");
    let text_raw_start = text_sec.raw_data_ptr as usize;
    let text_raw_end = text_raw_start + text_sec.raw_data_size as usize;
    let text_vaddr_start = vaddr_base + text_sec.virtual_address as u64;
    let text_bytes = &pe_data[text_raw_start..text_raw_end.min(pe_data.len())];

    println!("\n[*] Scanning .text section (0x{:08X} - 0x{:08X}) for calls to VdInitializeEngines & Dashboard Init:",
        text_vaddr_start, text_vaddr_start + text_sec.virtual_size as u64
    );

    use xenon_emu::kernel::KernelDispatcher;
    use xenon_emu::memory::MemoryBus;
    let mut bus = MemoryBus::new();
    pe.load_into_memory(&pe_data, vaddr_base, &mut bus)?;
    let thunk_map = dash_xex.setup_import_thunks(&mut bus)?;
    let kernel = KernelDispatcher::new();

    println!("\n[*] Resolving exact thunks for Video/Graphics subsystem (xboxkrnl.exe):");
    let mut vd_thunks = Vec::new();
    for (&addr, (lib, ord)) in &thunk_map {
        if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
            if let Some(name) = kernel.resolve_xboxkrnl(*ord) {
                if name.starts_with("Vd") {
                    vd_thunks.push((addr, *ord, name));
                }
            }
        }
    }
    vd_thunks.sort_by_key(|k| k.0);
    for (addr, ord, name) in &vd_thunks {
        println!("    Thunk 0x{:08X} -> xboxkrnl.exe!{} (#0x{:04X} / {})", addr, name, ord, ord);
    }

    // Now scan for callers to each of these actual thunk addresses
    for &(vd_addr, ord, name) in &vd_thunks {
        let mut callers = Vec::new();
        for (idx, chunk) in text_bytes.chunks_exact(4).enumerate() {
            let insn = byteorder::BigEndian::read_u32(chunk);
            let pc = text_vaddr_start + (idx * 4) as u64;
            if (insn >> 26) == 18 && (insn & 1) == 1 {
                let mut li = ((insn >> 2) & 0x00FF_FFFF) as i32;
                if (li & 0x0080_0000) != 0 {
                    li |= !0x00FF_FFFF;
                }
                let target = (pc as i64 + (li as i64 * 4)) as u64;
                if target == vd_addr {
                    callers.push(pc);
                }
            }
        }
        if !callers.is_empty() {
            println!("\n[+] Found {} caller(s) to {} (0x{:08X} / #0x{:04X}):", callers.len(), name, vd_addr, ord);
            for c in callers.iter().take(4) {
                println!("    - Caller at 0x{:08X}", c);
                // Find prologue
                let mut fn_start = *c;
                while fn_start > text_vaddr_start {
                    let off = (fn_start - text_vaddr_start) as usize;
                    let insn = BigEndian::read_u32(&text_bytes[off..off + 4]);
                    if (insn & 0xFFFF_0000) == 0x9421_0000 || insn == 0x7D88_02A6 || insn == 0x7C08_02A6 {
                        println!("      -> Function prologue: 0x{:08X} (opcode 0x{:08X})", fn_start, insn);
                        break;
                    }
                    fn_start -= 4;
                }
            }
        }
    }

    println!("\n[*] Scanning for callers to Video Master Init (0x9210EA48):");
    let mut ea_callers = Vec::new();
    for (idx, chunk) in text_bytes.chunks_exact(4).enumerate() {
        let insn = byteorder::BigEndian::read_u32(chunk);
        let pc = text_vaddr_start + (idx * 4) as u64;
        if (insn >> 26) == 18 && (insn & 1) == 1 {
            let mut li = ((insn >> 2) & 0x00FF_FFFF) as i32;
            if (li & 0x0080_0000) != 0 {
                li |= !0x00FF_FFFF;
            }
            let target = (pc as i64 + (li as i64 * 4)) as u64;
            if target == 0x9210EA48 {
                ea_callers.push(pc);
            }
        }
    }
    println!("[+] Found {} caller(s) to 0x9210EA48:", ea_callers.len());
    for c in &ea_callers {
        println!("    - Caller at 0x{:08X}", c);
        // Find prologue
        let mut fn_start = *c;
        while fn_start > text_vaddr_start {
            let off = (fn_start - text_vaddr_start) as usize;
            let insn = BigEndian::read_u32(&text_bytes[off..off + 4]);
            if (insn & 0xFFFF_0000) == 0x9421_0000 || insn == 0x7D88_02A6 || insn == 0x7C08_02A6 {
                println!("      -> Function prologue: 0x{:08X} (opcode 0x{:08X})", fn_start, insn);
                break;
            }
            fn_start -= 4;
        }
    }

    Ok(())
}
