use byteorder::{BigEndian, ByteOrder};
use std::fs;
use xenon_emu::loader::XexFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dash_bytes = fs::read("../SystemUpdate_17559_USB/$SystemUpdate/dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let img = dash_xex.decompress_image(&decrypted)?;
    let base: u64 = dash_xex.image_base as u64;

    let ranges: [(&str, u64, u64); 2] = [
        ("List-walk function", 0x92198820, 0x921988B8),
        ("Callback", 0x9219ED10, 0x9219ED30),
    ];
    for (name, s, e) in ranges {
        println!("\n=== {} ===", name);
        let mut a = s;
        while a < e {
            let off = (a - base) as usize;
            let raw = BigEndian::read_u32(&img[off..off + 4]);
            println!("0x{:08X}: 0x{:08X}", a, raw);
            a += 4;
        }
    }
    Ok(())
}
