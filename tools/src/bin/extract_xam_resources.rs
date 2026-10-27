//! Writes every resource of an XEX (XEX_HEADER_RESOURCE_INFO, e.g. xam.xex's `shrdres`, `xam`, `skin`) as
//! `<name>.xzp`, the local-file form Xenia's XamBuild*ResourceLocator points at (`file://media:/shrdres.xzp#...`).
//! Usage: extract_xam_resources <xam.xex> <output dir>. The output is console data: keep it outside the repository.

use xenon_emu::loader::xex::XexFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: extract_xam_resources <xam.xex> <output dir>");
        std::process::exit(2);
    }
    let data = std::fs::read(&args[1])?;
    let xex = XexFile::parse(&data)?;
    let image = xex.decompress_image(&xex.decrypt_image_payload(&data)?)?;
    std::fs::create_dir_all(&args[2])?;
    for r in &xex.resources {
        let name = String::from_utf8_lossy(r.name_bytes()).to_string();
        match r.bytes(&image, xex.image_base) {
            Some(bytes) => {
                let path = std::path::Path::new(&args[2]).join(format!("{}.xzp", name));
                std::fs::write(&path, bytes)?;
                println!("{:<8} 0x{:08X} {:>7} bytes -> {}", name, r.address, bytes.len(), path.display());
            }
            None => println!("{:<8} 0x{:08X} outside the image, skipped", name, r.address),
        }
    }
    Ok(())
}
