fn main() -> Result<(), Box<dyn std::error::Error>> {
    let img = image::open("../extracted_xui/named/xboxLogo.png")?;
    // The sphere is located around (75, 0) with width 150, height 126
    let sub = img.crop_imm(75, 0, 150, 126);
    sub.save("../extracted_xui/named/guide_jewel.png")?;
    println!("[+] Successfully created guide_jewel.png (150x126) from authentic xboxLogo.png!");
    Ok(())
}
