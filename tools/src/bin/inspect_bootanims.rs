use std::fs;
use xenon_emu::loader::XexFile;

fn main() {
    let files = [
        "../bootanim_00318000.xex",
        "../bootanim_00348000.xex",
        "../bootanim_003FC000.xex",
        "../bootanim_0040C000.xex",
        "../bootanim_00414000.xex",
        "../bootanim_00680000.xex",
    ];

    for path in &files {
        println!("==================================================");
        println!("Checking: {}", path);
        if let Ok(data) = fs::read(path) {
            match XexFile::parse(&data) {
                Ok(xex) => {
                    if let Some(exec) = &xex.execution_info {
                        println!("  Media ID:     {:08X}", exec.media_id);
                        println!("  Version:      {:08X}", exec.version);
                        println!("  Base Version: {:08X}", exec.base_version);
                        println!("  Title ID:     {:08X}", exec.title_id);
                    }
                    println!("  Load Address: 0x{:08X}", xex.load_address);
                    println!("  Entry Point:  0x{:08X}", xex.entry_point);
                    println!("  Image Size:   0x{:08X} ({} KB)", xex.image_size, xex.image_size / 1024);
                    println!("  Encryption:   0x{:04X}", xex.encryption_type);
                    println!("  Compression:  0x{:04X}", xex.compression_type);
                    println!("  Imports:      {} libraries", xex.import_libraries.len());
                    for lib in &xex.import_libraries {
                        println!("    - {} ({} records)", lib.name, lib.import_table.len());
                    }
                }
                Err(e) => println!("  Failed to parse XEX: {}", e),
            }
        } else {
            println!("  File not found!");
        }
    }
}
