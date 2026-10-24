use std::fs;
use xenon_emu::loader::{StfsPackage, XexFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Inspect $flash_bootanim.xexp from Dump Package    ");
    println!("========================================================");

    let su_path = "../$SystemUpdate/su20076000_00000000";
    let su_bytes = fs::read(su_path)?;
    let stfs = StfsPackage::parse(&su_bytes)?;

    println!("[*] Extracting '$flash_bootanim.xexp'...");
    let bootanim_bytes = stfs.extract_file(&su_bytes, "$flash_bootanim.xexp")?;
    println!("[+] Extracted {} bytes!", bootanim_bytes.len());

    println!("[*] First 64 bytes header:");
    for chunk in bootanim_bytes[..64.min(bootanim_bytes.len())].chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
        let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
        println!("  {:<48} | {}", hex.join(" "), ascii);
    }

    match XexFile::parse(&bootanim_bytes) {
        Ok(xex) => {
            println!("[+] Successfully parsed $flash_bootanim.xexp as XEX2!");
            xex.print_summary();
        }
        Err(e) => {
            println!("[!] Parse error for $flash_bootanim.xexp: {}", e);
        }
    }

    Ok(())
}
