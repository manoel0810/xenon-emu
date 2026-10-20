use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("         Deep Inspection of XUIB in BladeSkin.xur       ");
    println!("========================================================");

    let path = "../extracted_xui/BladeSkin.xur";
    let data = fs::read(path)?;

    // Find "XUIB" magic
    let mut xuib_off = None;
    for i in 0..data.len().saturating_sub(4) {
        if &data[i..i+4] == b"XUIB" {
            xuib_off = Some(i);
            break;
        }
    }

    if let Some(off) = xuib_off {
        println!("[+] Found 'XUIB' header at offset 0x{:08X}", off);
        let xuib = &data[off..];
        println!("[*] XUIB Header (first 64 bytes):");
        for chunk in xuib[..64.min(xuib.len())].chunks(16) {
            let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
            let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
            println!("  {:<48} | {}", hex.join(" "), ascii);
        }

        let version = BigEndian::read_u32(&xuib[4..8]);
        let total_len = BigEndian::read_u32(&xuib[8..12]);
        println!("[*] Version: 0x{:08X} | Length: {} bytes", version, total_len);

        // Scan for readable strings, scene names, and class names in XUIB
        println!("\n[*] Scanning for XUI Visual Tree Nodes & Classes:");
        let mut cur = 12;
        while cur < xuib.len() {
            // Check for ASCII / UTF-8 identifier
            if xuib[cur].is_ascii_alphabetic() {
                let mut end = cur;
                while end < xuib.len() && (xuib[end].is_ascii_alphanumeric() || xuib[end] == b'_' || xuib[end] == b':') {
                    end += 1;
                }
                let len = end - cur;
                if len >= 4 {
                    if let Ok(s) = std::str::from_utf8(&xuib[cur..end]) {
                        if s.starts_with("Xui") || s.contains("Scene") || s.contains("Blade") || s.contains("Button") || s.contains("Text") || s.contains("Image") || s.contains("Nav") || s.contains("Guide") || s.contains("Tab") {
                            println!("    -> Offset 0x{:04X}: '{}'", off + cur, s);
                        }
                    }
                }
                cur = end;
            } else {
                cur += 1;
            }
        }
    } else {
        println!("[!] 'XUIB' header not found!");
    }

    Ok(())
}
