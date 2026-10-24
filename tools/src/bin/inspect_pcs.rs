use byteorder::{BigEndian, ByteOrder};
use std::fs;
use xenon_emu::loader::XexFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dash_bytes = fs::read("../SystemUpdate_17559_USB/$SystemUpdate/dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let img = dash_xex.decompress_image(&decrypted)?;
    let base: u64 = dash_xex.image_base as u64; // 0x92000000

    let dis = |start: u64, end: u64, name: &str| {
        println!("\n=== {} (0x{:08X}..0x{:08X}) ===", name, start, end);
        let mut a = start;
        while a < end {
            let off = (a - base) as usize;
            let raw = BigEndian::read_u32(&img[off..off + 4]);
            println!("0x{:08X}: 0x{:08X}", a, raw);
            a += 4;
        }
    };

    let _find_callers = |target: u64, name: &str| {
        println!("\n=== Callers of {} (0x{:08X}) ===", name, target);
        let mut a = base;
        while a < base + img.len() as u64 - 4 {
            let off = (a - base) as usize;
            let raw = BigEndian::read_u32(&img[off..off + 4]);
            // bl opcode 18: [010010] [li 24 bits] [aa 1] [lk 1]
            if (raw >> 26) == 18 && (raw & 1) == 1 {
                let mut li = (raw & 0x03FF_FFFC) as i64;
                if (li & 0x0200_0000) != 0 {
                    li -= 0x0400_0000;
                }
                let dest = (a as i64 + li) as u64;
                if dest == target {
                    println!("  Caller at 0x{:08X} (raw 0x{:08X})", a, raw);
                }
            }
            a += 4;
        }
    };

    dis(0x9238BAA0, 0x9238BB00, "Function entry around 0x9238BAA0");
    dis(0x9238BB00, 0x9238BB50, "Function around 0x9238BB1C");

    Ok(())
}
