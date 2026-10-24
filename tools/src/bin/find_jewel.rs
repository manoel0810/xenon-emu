use std::fs;
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new("../extracted_xui/genuine_assets");
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with(".png") {
            // parse WxH from filename
            if let Some(pos) = name.rfind('_') {
                let dim_part = &name[pos+1..name.len()-4];
                if let Some(x_pos) = dim_part.find('x') {
                    let w: u32 = dim_part[..x_pos].parse().unwrap_or(0);
                    let h: u32 = dim_part[x_pos+1..].parse().unwrap_or(0);
                    if (w == 64 || w == 48 || w == 32 || w == 128 || w == 96) && w == h {
                        let len = entry.metadata()?.len();
                        println!("Icon: {} ({}x{}) [{} bytes]", name, w, h, len);
                    }
                }
            }
        }
    }
    Ok(())
}
