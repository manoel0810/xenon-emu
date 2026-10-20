use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = "../extracted_xui/dashmain.xur";
    let data = fs::read(path)?;

    // Find XUIB
    let mut xuib_off = 0;
    for i in 0..data.len().saturating_sub(4) {
        if &data[i..i+4] == b"XUIB" {
            xuib_off = i;
            break;
        }
    }
    let xuib = &data[xuib_off..];

    // Find VECT
    if let Some(pos) = xuib.windows(4).position(|w| w == b"VECT") {
        println!("[+] Found VECT at offset 0x{:04X}", pos);
        let size = BigEndian::read_u32(&xuib[pos+4..pos+8]) as usize;
        println!("[+] VECT size: {} bytes", size);
        let vdata = &xuib[pos+8..pos+8+size];
        for (idx, chunk) in vdata.chunks_exact(16).enumerate() {
            let x = BigEndian::read_f32(&chunk[0..4]);
            let y = BigEndian::read_f32(&chunk[4..8]);
            let z = BigEndian::read_f32(&chunk[8..12]);
            let w = BigEndian::read_f32(&chunk[12..16]);
            if idx < 20 {
                println!("  VECT[{:02}]: ({:.1}, {:.1}, {:.1}, {:.1})", idx, x, y, z, w);
            }
        }
    }

    Ok(())
}
