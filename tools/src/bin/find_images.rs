use std::fs;
use xenon_emu::loader::{StfsPackage, XexFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let payload = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let pe = dash_xex.decompress_image(&payload)?;

    println!("=== Searching for PNG / JPG images in Dashboard PE ({} bytes) ===", pe.len());

    let png_magic = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut png_count = 0;
    for i in 0..pe.len().saturating_sub(8) {
        if &pe[i..i+8] == &png_magic {
            println!("  [PNG #{:02}] Offset: 0x{:08X}", png_count + 1, i);
            png_count += 1;
        }
    }
    println!("[+] Found {} genuine PNG files inside Dashboard PE!", png_count);

    let jpg_magic = [0xFF, 0xD8, 0xFF];
    let mut jpg_count = 0;
    for i in 0..pe.len().saturating_sub(3) {
        if &pe[i..i+3] == &jpg_magic {
            if i + 4 < pe.len() && (pe[i+3] == 0xE0 || pe[i+3] == 0xE1 || pe[i+3] == 0xEE) {
                println!("  [JPG #{:02}] Offset: 0x{:08X}", jpg_count + 1, i);
                jpg_count += 1;
            }
        }
    }
    println!("[+] Found {} genuine JPEG files inside Dashboard PE!", jpg_count);

    Ok(())
}
