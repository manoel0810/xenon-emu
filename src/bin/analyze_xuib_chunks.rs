use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = "../extracted_xui/BladeSkin.xur";
    let data = fs::read(path)?;

    let mut xuib_off = 0;
    for i in 0..data.len().saturating_sub(4) {
        if &data[i..i+4] == b"XUIB" {
            xuib_off = i;
            break;
        }
    }

    println!("========================================================");
    println!("     XUIB FourCC Chunk Analyzer (Offset 0x{:08X})       ", xuib_off);
    println!("========================================================");

    let xuib = &data[xuib_off..];
    // Find all uppercase 4-letter chunks
    let mut i = 0;
    while i + 8 <= xuib.len() {
        let tag = &xuib[i..i+4];
        if tag.iter().all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
            let chunk_size = BigEndian::read_u32(&xuib[i+4..i+8]);
            let name = std::str::from_utf8(tag).unwrap_or("????");
            println!("  [Offset 0x{:04X}] Chunk: '{}' | Size: {} bytes (0x{:X})", i, name, chunk_size, chunk_size);
            
            if name == "STRN" {
                // String table
                println!("    -> String Table Content Preview:");
                let strn_data = &xuib[i+8..(i+8+chunk_size as usize).min(xuib.len())];
                for s in strn_data.split(|&b| b == 0) {
                    if let Ok(st) = std::str::from_utf8(s) {
                        let trimmed = st.trim();
                        if trimmed.len() >= 2 {
                            println!("       \"{}\"", trimmed);
                        }
                    }
                }
            }
        }
        i += 1;
    }

    Ok(())
}
