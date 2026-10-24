use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let images = [
        "../extracted_xui/xboxLogo.png",
        "../extracted_xui/ico_32x_console.png",
        "../extracted_xui/themes.jpg",
    ];

    for img_path in &images {
        if let Ok(bytes) = fs::read(img_path) {
            println!("=== Header of {} ({} bytes) ===", img_path, bytes.len());
            for chunk in bytes[..32.min(bytes.len())].chunks(16) {
                let hex: Vec<String> = chunk.iter().map(|b| format!("{:02X}", b)).collect();
                let ascii: String = chunk.iter().map(|&b| if b >= 32 && b <= 126 { b as char } else { '.' }).collect();
                println!("  {:<48} | {}", hex.join(" "), ascii);
            }
        }
    }

    Ok(())
}
