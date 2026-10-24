use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let src_dir = Path::new("../extracted_xui/genuine_assets");
    let dest_dir = Path::new("../extracted_xui/named");
    fs::create_dir_all(dest_dir)?;

    let mappings = [
        ("dash_icon_193_300x196.png", "xboxLogo.png"),
        ("dash_icon_208_219x127.png", "jewel.png"),
        ("dash_icon_211_400x136.png", "livelogo.png"),
        ("dash_icon_202_32x32.png", "ico_32x_console.png"),
        ("dash_icon_203_32x32.png", "ico_32x_network.png"),
        ("dash_icon_204_32x32.png", "ico_32x_parentalControl.png"),
        ("dash_icon_205_32x32.png", "ico_32x_Profiles.png"),
        ("dash_icon_206_32x32.png", "ico_32x_systemSet.png"),
        ("dash_icon_106_64x64.png", "ico_64x_Game.png"),
        ("dash_icon_180_64x64.png", "ico_64x_games.png"),
        ("dash_icon_178_64x64.png", "ico_64x_music.png"),
        ("dash_icon_112_64x64.png", "ico_64x_System.png"),
        ("dash_icon_113_64x64.png", "ico_64x_Trailers.png"),
        ("dash_art_06_430x80.jpg", "themes.jpg"),
        ("dash_art_09_430x80.jpg", "lastPlayed.jpg"),
        ("dash_art_10_430x80.jpg", "myArcadeGames.jpg"),
        ("dash_art_05_430x80.jpg", "media.jpg"),
        ("dash_art_01_146x125.jpg", "yak1.jpg"),
    ];

    for (src, dst) in &mappings {
        let s = src_dir.join(src);
        let d = dest_dir.join(dst);
        if s.exists() {
            fs::copy(&s, &d)?;
            println!("[+] Copied {} -> {}", src, dst);
        }
    }

    Ok(())
}
