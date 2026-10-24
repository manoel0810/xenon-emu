use std::fs;
use byteorder::{BigEndian, ByteOrder};

fn extract_xuib_strings(xuib: &[u8]) -> Vec<String> {
    let mut strings = Vec::new();
    let pattern = [0x00, 0x58, 0x00, 0x75, 0x00, 0x69];
    if let Some(pos) = xuib.windows(6).position(|w| w == pattern) {
        let mut s_idx = pos.saturating_sub(2);
        while s_idx + 2 <= xuib.len() {
            let char_count = BigEndian::read_u16(&xuib[s_idx..s_idx + 2]) as usize;
            if char_count == 0 || char_count > 512 {
                break;
            }
            s_idx += 2;
            if s_idx + char_count * 2 <= xuib.len() {
                let utf16_chars: Vec<u16> = xuib[s_idx..s_idx + char_count * 2]
                    .chunks_exact(2)
                    .map(|b| BigEndian::read_u16(b))
                    .collect();
                let s = String::from_utf16_lossy(&utf16_chars).trim().to_string();
                if !s.is_empty() {
                    for sub in s.split('\0') {
                        let sub_t = sub.trim().to_string();
                        if !sub_t.is_empty() {
                            strings.push(sub_t);
                        }
                    }
                }
                s_idx += char_count * 2;
            } else {
                break;
            }
        }
    }
    strings
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenes = [
        "dashmain.xur",
        "BladeSkin.xur",
        "gamesSignedOut.xur",
        "mediaSignedOut.xur",
        "skin.xur",
    ];

    for s in &scenes {
        let path = format!("../extracted_xui/{}", s);
        if let Ok(bytes) = fs::read(&path) {
            let strings = extract_xuib_strings(&bytes);
            println!("\n[+] Scene '{}' ({} bytes): Extracted {} authentic strings", s, bytes.len(), strings.len());
            for (idx, str_val) in strings.iter().take(8).enumerate() {
                println!("    [{:02}] \"{}\"", idx + 1, str_val);
            }
        }
    }

    Ok(())
}
