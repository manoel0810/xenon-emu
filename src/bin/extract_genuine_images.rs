use std::fs;
use std::path::Path;
use xenon_emu::loader::{StfsPackage, XexFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let payload = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let pe = dash_xex.decompress_image(&payload)?;

    let out_dir = Path::new("../extracted_xui/genuine_assets");
    fs::create_dir_all(out_dir)?;

    println!("=== Extracting All 212 Authentic PNGs from Dashboard PE ===");
    let png_magic = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let mut png_idx = 0;
    for i in 0..pe.len().saturating_sub(8) {
        if &pe[i..i+8] == &png_magic {
            // Find IEND chunk
            if let Some(iend_rel) = pe[i..].windows(8).position(|w| &w[4..8] == b"IEND") {
                let png_len = iend_rel + 12; // includes 4-byte CRC
                let png_data = &pe[i..i + png_len];
                
                // Validate with image crate
                if let Ok(dyn_img) = image::load_from_memory(png_data) {
                    png_idx += 1;
                    let file_name = format!("dash_icon_{:03}_{}x{}.png", png_idx, dyn_img.width(), dyn_img.height());
                    fs::write(out_dir.join(&file_name), png_data)?;
                    println!("  [+] Valid PNG #{:03}: {} ({}x{}) [{} bytes]", png_idx, file_name, dyn_img.width(), dyn_img.height(), png_len);
                }
            }
        }
    }

    println!("\n=== Extracting All Authentic JPEGs from Dashboard PE ===");
    let jpg_magic = [0xFF, 0xD8, 0xFF];
    let mut jpg_idx = 0;
    for i in 0..pe.len().saturating_sub(4) {
        if &pe[i..i+3] == &jpg_magic && (pe[i+3] == 0xE0 || pe[i+3] == 0xE1 || pe[i+3] == 0xEE) {
            // Find EOI marker (FF D9)
            if let Some(eoi_rel) = pe[i..].windows(2).position(|w| w == [0xFF, 0xD9]) {
                let jpg_len = eoi_rel + 2;
                let jpg_data = &pe[i..i + jpg_len];
                if let Ok(dyn_img) = image::load_from_memory(jpg_data) {
                    jpg_idx += 1;
                    let file_name = format!("dash_art_{:02}_{}x{}.jpg", jpg_idx, dyn_img.width(), dyn_img.height());
                    fs::write(out_dir.join(&file_name), jpg_data)?;
                    println!("  [+] Valid JPG #{:02}: {} ({}x{}) [{} bytes]", jpg_idx, file_name, dyn_img.width(), dyn_img.height(), jpg_len);
                }
            }
        }
    }

    println!("\n[+] Total Authentic Assets Extracted: {} PNGs, {} JPGs to '{}'", png_idx, jpg_idx, out_dir.display());

    Ok(())
}
