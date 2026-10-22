use byteorder::{BigEndian, ByteOrder};
use std::fs;
use xenon_emu::loader::{PeImage, XexFile};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let metro_path = "../SystemUpdate_17559_USB/$SystemUpdate/dash.xex";
    let dash_bytes = fs::read(metro_path)?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let uncompressed = dash_xex.decompress_image(&decrypted)?;
    let _pe = PeImage::parse(&uncompressed)?;

    let vaddr_base: u64 = dash_xex.image_base as u64; // 0x92000000

    let target = 0x9293A4ECu64;
    println!("\n=== Inspecting Target Function at 0x{:08X} ===", target);
    let rva = (target - vaddr_base) as usize;
    for curr in (rva..rva + 40).step_by(4) {
        let addr = vaddr_base + curr as u64;
        let raw = BigEndian::read_u32(&uncompressed[curr..curr + 4]);
        println!("0x{:08X}: 0x{:08X} (opcode: 0x{:02X})", addr, raw, raw >> 26);
    }

    Ok(())
}
