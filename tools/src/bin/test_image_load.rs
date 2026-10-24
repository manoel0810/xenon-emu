use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let images = [
        "../extracted_xui/named/xboxLogo.png",
        "../extracted_xui/named/jewel.png",
        "../extracted_xui/named/livelogo.png",
        "../extracted_xui/named/ico_32x_console.png",
        "../extracted_xui/named/ico_64x_games.png",
        "../extracted_xui/named/themes.jpg",
        "../extracted_xui/named/media.jpg",
    ];

    for img_path in &images {
        let p = Path::new(img_path);
        if p.exists() {
            let img = image::open(p)?;
            println!("[+] Loaded '{}' -> Dimensions: {}x{}, Color: {:?}",
                img_path, img.width(), img.height(), img.color()
            );
        } else {
            println!("[!] File not found: {}", img_path);
        }
    }

    Ok(())
}
