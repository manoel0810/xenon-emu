use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read("../extracted_xui/dashmain.xur")?;
    println!("=== Testing dashmain.xur XUIZ offsets ===");
    let data_ptr = BigEndian::read_u32(&bytes[16..20]) as usize;
    let num_entries = BigEndian::read_u16(&bytes[20..22]) as usize;
    println!("Data Ptr: 0x{:04X} ({}), Num Entries: {}", data_ptr, data_ptr, num_entries);

    let mut bufpos = 22usize;
    for i in 0..num_entries {
        let flen = BigEndian::read_u32(&bytes[bufpos..bufpos + 4]) as usize;
        let fptr = BigEndian::read_u32(&bytes[bufpos + 4..bufpos + 8]) as usize;
        let fnlen = bytes[bufpos + 8] as usize;
        bufpos += 9;

        let name_bytes = &bytes[bufpos..bufpos + fnlen * 2];
        bufpos += fnlen * 2;

        let u16_chars: Vec<u16> = name_bytes.chunks_exact(2).map(|c| BigEndian::read_u16(c)).collect();
        let name = String::from_utf16_lossy(&u16_chars);

        // Check if data is at fptr or data_ptr + fptr
        println!("\nEntry [{:02}] '{}' (len {})", i, name, flen);
        println!("  fptr = 0x{:08X}", fptr);
        println!("  data_ptr + fptr = 0x{:08X}", data_ptr + fptr);

        // Print first 8 bytes of both
        if fptr + 8 <= bytes.len() {
            println!("    At fptr:            {:02X?}", &bytes[fptr..fptr+8]);
        }
        if data_ptr + fptr + 8 <= bytes.len() {
            println!("    At data_ptr + fptr: {:02X?}", &bytes[data_ptr + fptr..data_ptr + fptr + 8]);
        }
    }

    Ok(())
}
