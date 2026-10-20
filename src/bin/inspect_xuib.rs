use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("         Deep Inspection of XUIB in BladeSkin.xur       ");
    println!("========================================================");

    let path = "../extracted_xui/dashmain.xur";
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

        println!("\n[*] Parsing XUIB chunks starting at offset 0x{:08X}:", off);
        let mut cur = 0;
        while cur + 8 <= xuib.len() {
            let tag = &xuib[cur..cur + 4];
            if tag.iter().all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
                let size = BigEndian::read_u32(&xuib[cur + 4..cur + 8]) as usize;
                let tag_str = std::str::from_utf8(tag).unwrap_or("????");
                println!("  [Chunk] '{}' at relative offset 0x{:04X} (size {} / 0x{:X} bytes)", tag_str, cur, size, size);
                
                let start = cur + 8;
                let end = (start + size).min(xuib.len());
                let cdata = &xuib[start..end];

        // Search for string table start (00 09 00 58 00 75 00 69)
        let pattern = [0x00, 0x09, 0x00, 0x58, 0x00, 0x75, 0x00, 0x69];
        if let Some(pos) = xuib.windows(8).position(|w| w == pattern) {
            println!("\n[+] Found UTF-16BE String Table at relative offset 0x{:04X} (file offset 0x{:08X}):", pos, off + pos);
            let mut s_idx = pos;
            let mut count = 0;
            while s_idx + 2 <= xuib.len() {
                let char_count = BigEndian::read_u16(&xuib[s_idx..s_idx+2]) as usize;
                if char_count == 0 || char_count > 256 {
                    break;
                }
                s_idx += 2;
                if s_idx + char_count * 2 <= xuib.len() {
                    let utf16_slice: Vec<u16> = xuib[s_idx..s_idx + char_count * 2]
                        .chunks_exact(2)
                        .map(|b| BigEndian::read_u16(b))
                        .collect();
                    let s = String::from_utf16_lossy(&utf16_slice);
                    println!("  [{:03}] (len {:2}): \"{}\"", count, char_count, s);
                    count += 1;
                    s_idx += char_count * 2;
                } else {
                    break;
                }
            }
            println!("[+] Successfully decoded {} strings from XUIB string table!", count);
        }
                cur += 8 + size;
            } else {
                cur += 1;
            }
        }
    }

    Ok(())
}
