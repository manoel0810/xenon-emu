use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xbox 360 String Table (.xus) Analyzer              ");
    println!("========================================================");

    let path = "../extracted_xui/pt-br/Strings.xus";
    let data = fs::read(path)?;
    println!("[+] Read {} bytes of pt-br/Strings.xus", data.len());

    // The .xus format contains UTF-16 BE strings with string IDs!
    // Let's decode UTF-16 BE characters:
    let u16_chars: Vec<u16> = data.chunks_exact(2).map(|c| BigEndian::read_u16(c)).collect();
    let text = String::from_utf16_lossy(&u16_chars);

    println!("\n[*] Recovered Strings in Portuguese (pt-br):");
    let mut count = 0;
    for line in text.split('\0') {
        let t = line.trim();
        if t.len() >= 3 && t.chars().any(|c| c.is_alphabetic()) {
            println!("  [{:03}] {}", count, t);
            count += 1;
            if count >= 30 {
                break;
            }
        }
    }

    Ok(())
}
