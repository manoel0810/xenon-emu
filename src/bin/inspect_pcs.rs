use byteorder::{BigEndian, ByteOrder};
use std::fs;
use xenon_emu::loader::XexFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dash_bytes = fs::read("../SystemUpdate_17559_USB/$SystemUpdate/dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let img = dash_xex.decompress_image(&decrypted)?;
    let base: u64 = dash_xex.image_base as u64; // 0x92000000

    let start = 0x92194D14u64;
    let end = 0x92194E10u64;
    println!("=== Disassembly of 0x{:08X}..0x{:08X} ===", start, end);
    let mut a = start;
    while a < end {
        let off = (a - base) as usize;
        let raw = BigEndian::read_u32(&img[off..off + 4]);
        println!("0x{:08X}: 0x{:08X}", a, raw);
        a += 4;
    }
    Ok(())
}

