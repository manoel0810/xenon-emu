use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Extracting bootanim.xex from Corona NAND Dump ===");
    let nand_path = "../Corona 16mb/Corona_053679523709_1685af93d3ac27ccd76b0c82a67d95b2.bin";
    let nand_data = fs::read(nand_path)?;
    println!("[*] Read {} bytes from NAND", nand_data.len());

    let page_data = 512;
    let page_spare = 16;
    let page_raw = page_data + page_spare; // 528
    let pages_per_block = 32;
    let block_raw = pages_per_block * page_raw; // 16896 = 0x4200
    let block_data = pages_per_block * page_data; // 16384 = 0x4000

    // Remove spare bytes from entire NAND to get pure data stream
    println!("[*] Stripping ECC spare data (512/16)...");
    let num_pages = nand_data.len() / page_raw;
    let mut pure_nand = Vec::with_capacity(num_pages * page_data);
    for p in 0..num_pages {
        let p_start = p * page_raw;
        pure_nand.extend_from_slice(&nand_data[p_start..p_start + page_data]);
    }
    println!("[+] Pure NAND data size: {} bytes ({} MB)", pure_nand.len(), pure_nand.len() / (1024*1024));

    // Search for "XEX2" in pure NAND
    println!("\n[*] Scanning pure NAND for 'XEX2' executables:");
    for i in 0..pure_nand.len().saturating_sub(4) {
        if &pure_nand[i..i+4] == b"XEX2" {
            // Check if valid XEX2 header
            let header_size = BigEndian::read_u32(&pure_nand[i+4..i+8]);
            let security_offset = BigEndian::read_u32(&pure_nand[i+8..i+12]);
            if header_size > 0 && header_size < 0x10000 && security_offset < 0x1000 {
                println!("  [XEX2 Found] Pure Offset: 0x{:08X} (Block 0x{:03X}) | Header: 0x{:X}, SecOff: 0x{:X}",
                    i, i / 0x4000, header_size, security_offset
                );
            }
        }
    }

    println!("\n[*] Searching pure NAND for FFS directory entries:");
    for i in 0..pure_nand.len().saturating_sub(64) {
        let name_slice = &pure_nand[i..i+32];
        if let Ok(name) = std::str::from_utf8(name_slice) {
            let null_term = name.split('\0').next().unwrap_or("");
            if (null_term.ends_with(".xex") || null_term.ends_with(".bin")) && null_term.len() >= 4 && null_term.len() <= 24 {
                println!("  [FFS Entry at 0x{:08X}] Filename: '{}'", i, null_term);
                let start = i.saturating_sub(16);
                let end = (i + 48).min(pure_nand.len());
                println!("    Hex: {:02X?}", &pure_nand[start..end]);
            }
        }
    }

    Ok(())
}
