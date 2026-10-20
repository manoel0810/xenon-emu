use byteorder::{BigEndian, ByteOrder};
use std::fs;
use std::path::Path;
use xenon_emu::loader::{StfsPackage, XexFile, XzpPackage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     XUI Asset Extractor & Binary Structure Parser     ");
    println!("========================================================");

    let su_path = "../$SystemUpdate/su20076000_00000000";
    println!("[*] Loading STFS container: {}", su_path);
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;

    println!("[*] Extracting '$flash_dash.xex'...");
    let dash_bytes = stfs.extract_file(&su_bytes, "$flash_dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;

    println!("[*] Decrypting and decompressing PE image...");
    let payload = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let uncompressed_pe = dash_xex.decompress_image(&payload)?;

    let out_dir = Path::new("../extracted_xui");
    if !out_dir.exists() {
        fs::create_dir_all(out_dir)?;
    }

    // Find all XUIZ occurrences
    let mut xzp_offsets = Vec::new();
    for i in 0..uncompressed_pe.len().saturating_sub(4) {
        if &uncompressed_pe[i..i + 4] == b"XUIZ" {
            xzp_offsets.push(i);
        }
    }
    println!("[+] Found {} embedded XUIZ (.xzp) packages in Dashboard PE image!", xzp_offsets.len());

    let mut total_extracted = 0;
    for (pkg_idx, &offset) in xzp_offsets.iter().enumerate() {
        if offset + 22 > uncompressed_pe.len() {
            continue;
        }

        match XzpPackage::parse(&uncompressed_pe[offset..]) {
            Ok(xzp) => {
                println!("\n[*] Package #{:02} at PE offset 0x{:08X} ({} entries, {} bytes):",
                    pkg_idx + 1, offset, xzp.entries.len(), xzp.total_size
                );

                for entry in &xzp.entries {
                    if entry.name.is_empty() || entry.data.is_empty() {
                        continue;
                    }

                    // Create subdirectories if needed (e.g., pt-br/strings.xus)
                    let sanitized_name = entry.name.replace('\\', "/");
                    let target_path = out_dir.join(&sanitized_name);
                    if let Some(parent) = target_path.parent() {
                        fs::create_dir_all(parent)?;
                    }

                    fs::write(&target_path, &entry.data)?;
                    total_extracted += 1;
                    println!("    -> Extracted: {:<38} ({} bytes)", sanitized_name, entry.data.len());
                }
            }
            Err(e) => {
                // Some occurrences might be nested or partial
                // println!("    Skipping offset 0x{:08X}: {}", offset, e);
            }
        }
    }

    println!("\n[+] Total assets successfully extracted: {} files to '{}'", total_extracted, out_dir.display());

    // Now let's inspect BladeSkin.xur
    let bladeskin_path = out_dir.join("BladeSkin.xur");
    if bladeskin_path.exists() {
        let xur_bytes = fs::read(&bladeskin_path)?;
        println!("\n========================================================");
        println!("     Binary Analysis of 'BladeSkin.xur' ({} bytes)      ", xur_bytes.len());
        println!("========================================================");

        println!("[*] First 64 bytes of BladeSkin.xur:");
        for chunk in xur_bytes[..64.min(xur_bytes.len())].chunks(16) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
            let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
            println!("  {:<48} | {}", hex.join(" "), ascii);
        }

        // Check XUR Magic: 'XUR\x00' or similar
        let magic = &xur_bytes[0..4];
        println!("[*] XUR Header Magic: {:02X?} (ASCII: '{}')", magic, String::from_utf8_lossy(magic));
    }

    // Inspect pt-br strings.xus
    let strings_pt = out_dir.join("pt-br/strings.xus");
    if strings_pt.exists() {
        let xus_bytes = fs::read(&strings_pt)?;
        println!("\n========================================================");
        println!("     Binary Analysis of 'pt-br/strings.xus' ({} bytes)  ", xus_bytes.len());
        println!("========================================================");

        println!("[*] First 64 bytes of strings.xus:");
        for chunk in xus_bytes[..64.min(xus_bytes.len())].chunks(16) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
            let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
            println!("  {:<48} | {}", hex.join(" "), ascii);
        }
    }

    Ok(())
}
