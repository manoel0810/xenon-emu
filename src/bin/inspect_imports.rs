use std::fs;
use xenon_emu::kernel::exports::{get_xam_name, get_xboxkrnl_name};
use xenon_emu::loader::{PeImage, StfsPackage, XexFile};
use xenon_emu::memory::MemoryBus;

fn main() -> Result<(), Box<dyn std::error::Error>> {
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

    let mut imports_by_lib: std::collections::BTreeMap<String, Vec<(u16, u64, &'static str)>> = std::collections::BTreeMap::new();

    for (&addr, (lib, ord)) in &thunk_map {
        let name = if lib.eq_ignore_ascii_case("xboxkrnl.exe") {
            get_xboxkrnl_name(*ord).unwrap_or("Unknown")
        } else {
            get_xam_name(*ord).unwrap_or("Unknown")
        };
        imports_by_lib.entry(lib.clone()).or_default().push((*ord, addr, name));
    }

    println!("========================================================");
    println!("     $flash_dash.xex Imported Function Manifest         ");
    println!("========================================================");
    for (lib, mut fns) in imports_by_lib {
        fns.sort_by_key(|f| f.0);
        println!("\n[+] Library '{}' ({} functions):", lib, fns.len());
        for (ord, addr, name) in fns {
            println!("    - Ord #0x{:04X} ({:<4}) @ 0x{:08X} => {}", ord, ord, addr, name);
        }
    }

    Ok(())
}
