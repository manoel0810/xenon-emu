use std::fs;
use xenon_emu::loader::XexFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Inspecting bootanim_00348000.xex ===");
    let bytes = fs::read("../bootanim_00348000.xex")?;
    let xex = XexFile::parse(&bytes)?;
    xex.print_summary();

    println!("\n[*] Attempting decryption and decompression of bootanim payload...");
    match xex.decrypt_image_payload(&bytes) {
        Ok(payload) => {
            println!("[+] Decrypted payload: {} bytes", payload.len());
            match xex.decompress_image(&payload) {
                Ok(pe) => {
                    println!("[+] Decompressed PE image: {} bytes", pe.len());
                    // Search for XUIZ or video / resources in bootanim
                    for i in 0..pe.len().saturating_sub(4) {
                        if &pe[i..i+4] == b"XUIZ" || &pe[i..i+4] == b"XUIB" || &pe[i..i+4] == b"BIK" {
                            let tag = std::str::from_utf8(&pe[i..i+4]).unwrap_or("????");
                            println!("  [Resource Found] Tag '{}' at PE offset 0x{:08X}", tag, i);
                        }
                    }
                }
                Err(e) => println!("[!] Decompression failed: {}", e),
            }
        }
        Err(e) => println!("[!] Decryption failed: {}", e),
    }

    Ok(())
}
