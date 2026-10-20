use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("     Xbox 360 Corona 16MB NAND Raw Dump Inspector       ");
    println!("========================================================");

    let nand_path = "../Corona 16mb/Corona_053679523709_1685af93d3ac27ccd76b0c82a67d95b2.bin";
    let nand_data = fs::read(nand_path)?;
    println!("[+] Read NAND dump: {} bytes ({} MB)", nand_data.len(), nand_data.len() / (1024 * 1024));

    // Check first 16 bytes (Flash Header)
    println!("\n[*] Flash Header (first 32 bytes):");
    for chunk in nand_data[..32].chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
        let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
        println!("  {:<48} | {}", hex.join(" "), ascii);
    }

    let magic = BigEndian::read_u16(&nand_data[0..2]);
    println!("[*] Magic: 0x{:04X} (Expected: 0xFF4F for Small Block / Corona)", magic);

    // Bootloader table starts at 0x8
    let cb_offset = BigEndian::read_u32(&nand_data[8..12]);
    println!("[*] 2BL (CB) offset: 0x{:08X}", cb_offset);

    println!("\n[*] Scanning NAND for Bootloaders (CB, CD, CE, CF, CG)...");
    let page_data_size = 512;
    let page_spare_size = 16;
    let page_total_size = page_data_size + page_spare_size; // 528
    let block_pages = 32;
    let block_size = block_pages * page_total_size; // 16896 bytes

    // Scan the first 100 blocks
    for b in 0..100 {
        let b_off = b * block_size;
        if b_off + 16 <= nand_data.len() {
            let sig = &nand_data[b_off..b_off + 2];
            if sig == b"CB" || sig == b"CD" || sig == b"CE" || sig == b"CF" || sig == b"CG" {
                let name = std::str::from_utf8(sig).unwrap_or("??");
                let build = BigEndian::read_u16(&nand_data[b_off + 2..b_off + 4]);
                let flags = BigEndian::read_u16(&nand_data[b_off + 4..b_off + 6]);
                let entry = BigEndian::read_u32(&nand_data[b_off + 6..b_off + 10]);
                let size = BigEndian::read_u32(&nand_data[b_off + 10..b_off + 14]);
                println!("  [Block 0x{:03X} / Offset 0x{:08X}] Bootloader: {} Build: {} | Entry: 0x{:08X} | Flags: 0x{:04X} | Size: 0x{:X}", 
                    b, b_off, name, build, entry, flags, size
                );
            }
        }
    }

    Ok(())
}
