use std::collections::HashMap;
use std::fs;
use xenon_emu::loader::{PeImage, StfsPackage, XexFile};
use xenon_emu::memory::MemoryBus;

fn load_table(path: &str) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if line.starts_with("XE_EXPORT(") {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() >= 3 {
                    let ord_str = parts[1].trim();
                    let name = parts[2].trim();
                    let ord = if ord_str.starts_with("0x") {
                        u32::from_str_radix(&ord_str[2..], 16).unwrap_or(0)
                    } else {
                        ord_str.parse().unwrap_or(0)
                    };
                    map.insert(ord, name.to_string());
                }
            }
        }
    }
    map
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let krnl_table = load_table("C:/Users/manoe/.gemini/antigravity-ide/brain/3f1ffb6b-9bf4-4cc0-b24f-7ff0bb70f26d/scratch/xboxkrnl_table.inc");
    let xam_table = load_table("C:/Users/manoe/.gemini/antigravity-ide/brain/3f1ffb6b-9bf4-4cc0-b24f-7ff0bb70f26d/scratch/xam_table.inc");

    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let pe_data = dash_xex.decompress_image(&decrypted)?;
    let pe = PeImage::parse(&pe_data)?;

    let mut bus = MemoryBus::new();
    let vaddr_base = dash_xex.load_address as u64;
    pe.load_into_memory(&pe_data, vaddr_base, &mut bus)?;

    let thunk_map = dash_xex.setup_import_thunks(&mut bus)?;

    for (addr, (lib, ord)) in &thunk_map {
        if *addr >= 0x9268E9A0 && *addr <= 0x9268E9F0 {
            let table = if lib.eq_ignore_ascii_case("xboxkrnl.exe") { &krnl_table } else { &xam_table };
            let name = table.get(&(*ord as u32)).cloned().unwrap_or("UNKNOWN".into());
            println!("  0x{:08X} -> {}!#{} ({})", addr, lib, ord, name);
        }
    }

    Ok(())
}
