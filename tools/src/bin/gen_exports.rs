use std::collections::BTreeMap;
use std::fs;

fn parse_inc(path: &str) -> BTreeMap<u16, String> {
    let mut map = BTreeMap::new();
    let content = fs::read_to_string(path).expect("failed to read table");
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("XE_EXPORT(") {
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 3 {
                let ord_str = parts[1].trim();
                let name = parts[2].trim();
                let ord = if ord_str.starts_with("0x") {
                    u32::from_str_radix(&ord_str[2..], 16).unwrap_or(0)
                } else {
                    ord_str.parse().unwrap_or(0)
                };
                if ord <= 0xFFFF {
                    map.insert(ord as u16, name.to_string());
                }
            }
        }
    }
    map
}

fn main() {
    let krnl = parse_inc("C:/Users/manoe/.gemini/antigravity-ide/brain/3f1ffb6b-9bf4-4cc0-b24f-7ff0bb70f26d/scratch/xboxkrnl_table.inc");
    let xam = parse_inc("C:/Users/manoe/.gemini/antigravity-ide/brain/3f1ffb6b-9bf4-4cc0-b24f-7ff0bb70f26d/scratch/xam_table.inc");

    println!("xboxkrnl exports: {}, xam exports: {}", krnl.len(), xam.len());

    let mut out = String::new();
    out.push_str("/// Canonical Xbox 360 Kernel & XAM Export Names (from Official Xenia Canary tables)\n\n");

    out.push_str("pub fn get_xboxkrnl_name(ord: u16) -> Option<&'static str> {\n");
    out.push_str("    match ord {\n");
    for (ord, name) in &krnl {
        out.push_str(&format!("        0x{:04X} => Some(\"{}\"),\n", ord, name));
    }
    out.push_str("        _ => None,\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("pub fn get_xam_name(ord: u16) -> Option<&'static str> {\n");
    out.push_str("    match ord {\n");
    for (ord, name) in &xam {
        out.push_str(&format!("        0x{:04X} => Some(\"{}\"),\n", ord, name));
    }
    out.push_str("        _ => None,\n");
    out.push_str("    }\n");
    out.push_str("}\n");

    fs::write("src/kernel/exports.rs", out).expect("failed to write exports.rs");
    println!("Successfully generated src/kernel/exports.rs!");
}
