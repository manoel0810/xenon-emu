use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Searching Corona 16MB NAND Dump ===");
    let nand_path = "../Corona 16mb/Corona_053679523709_1685af93d3ac27ccd76b0c82a67d95b2.bin";
    let nand = fs::read(nand_path)?;
    println!("[*] NAND size: {} bytes ({} MB)", nand.len(), nand.len() / (1024 * 1024));

    let xur_path = "../extracted_xui/dashmain.xur";
    let bytes = fs::read(xur_path)?;
    println!("[*] BladeSkin.xur size: {} bytes", bytes.len());
    for chunk in bytes[..256.min(bytes.len())].chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
        let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
        println!("  {:<48} | {}", hex.join(" "), ascii);
    }

    Ok(())
}
