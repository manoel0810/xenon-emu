use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let nand_path = "../Corona 16mb/Corona_053679523709_1685af93d3ac27ccd76b0c82a67d95b2.bin";
    let nand_data = fs::read(nand_path)?;

    let page_total_size = 528;
    let block_pages = 32;
    let block_size = block_pages * page_total_size; // 16896 bytes

    println!("========================================================");
    println!("     Scanning Flash File System (FFS) in 16MB NAND      ");
    println!("========================================================");

    // Xbox 360 File System Header usually begins around Block 0x40-0x80
    // File table entries contain: filename (null-terminated ascii), length, block pointer
    let mut found_files = 0;
    for b in 30..900 {
        let b_off = b * block_size;
        if b_off + 0x4000 > nand_data.len() {
            break;
        }

        // Check each 64-byte or 32-byte slot in the block for file names
        for slot in 0..16 {
            let s_off = b_off + slot * 64;
            if s_off + 32 <= nand_data.len() {
                let name_bytes = &nand_data[s_off..s_off + 20];
                if name_bytes[0].is_ascii_alphanumeric() || name_bytes[0] == b'$' {
                    let mut is_str = true;
                    let mut len = 0;
                    for &ch in name_bytes {
                        if ch == 0 {
                            break;
                        }
                        if !(ch.is_ascii_graphic() || ch == b' ' || ch == b'.' || ch == b'_' || ch == b'$') {
                            is_str = false;
                            break;
                        }
                        len += 1;
                    }
                    if is_str && len >= 3 {
                        if let Ok(name) = std::str::from_utf8(&name_bytes[..len]) {
                            if name.contains(".bin") || name.contains(".xex") || name.contains(".xexp") || name.contains(".txt") || name.contains("secdata") {
                                println!("  [Block 0x{:03X} / Offset 0x{:08X}] Found File Entry: '{}'", b, s_off, name);
                                found_files += 1;
                            }
                        }
                    }
                }
            }
        }
    }

    println!("[+] Scan complete! Found {} file system entries.", found_files);
    Ok(())
}
