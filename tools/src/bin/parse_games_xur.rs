use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = "../extracted_xui/gamesSignedOut.xur";
    let data = fs::read(path)?;

    println!("========================================================");
    println!("     Detailed Parser for gamesSignedOut.xur             ");
    println!("========================================================");

    let mut i = 0;
    while i + 4 <= data.len() {
        if &data[i..i+4] == b"XUIB" {
            println!("\n[*] Found XUIB block at offset 0x{:04X}", i);
            let version = BigEndian::read_u32(&data[i+4..i+8]);
            println!("    Version: 0x{:08X}", version);
            
            // Search inside this XUIB block for chunks
            let mut j = i + 12;
            while j + 8 <= data.len() && &data[j..j+4] != b"XUIB" {
                let tag = &data[j..j+4];
                if tag.iter().all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
                    let chunk_size = BigEndian::read_u32(&data[j+4..j+8]);
                    let tag_str = std::str::from_utf8(tag).unwrap_or("????");
                    println!("    [Chunk 0x{:04X}] '{}' (Size: {} bytes)", j, tag_str, chunk_size);

                    if tag_str == "STRN" {
                        let end = (j + 8 + chunk_size as usize).min(data.len());
                        let raw_strn = &data[j+8..end];
                        let u16_chars: Vec<u16> = raw_strn.chunks_exact(2).map(|c| BigEndian::read_u16(c)).collect();
                        let text = String::from_utf16_lossy(&u16_chars);
                        for s in text.split('\0') {
                            let trimmed = s.trim();
                            if !trimmed.is_empty() {
                                println!("        -> UTF-16 Node/Class: \"{}\"", trimmed);
                            }
                        }
                    }
                    j += 8 + (chunk_size as usize);
                } else {
                    j += 1;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }

    Ok(())
}
