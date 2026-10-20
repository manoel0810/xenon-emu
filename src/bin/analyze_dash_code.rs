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

    println!("[+] Found {} interesting strings:", found_strings.len());
    for (vaddr, s) in found_strings.iter().take(40) {
        println!("    0x{:016X} : \"{}\"", vaddr, s);
    }

    Ok(())
}
