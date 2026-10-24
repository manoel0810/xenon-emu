use std::fs;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let files = [
        "../extracted_xui/gamesSignedOut.xur",
        "../extracted_xui/mediaSignedOut.xur",
        "../extracted_xui/liveSignedOutUI.xur",
        "../extracted_xui/Status.xur",
    ];

    for path in &files {
        let data = fs::read(path)?;
        println!("========================================================");
        println!("  File: {} ({} bytes)", path, data.len());
        println!("========================================================");

        // Find printable ASCII / UTF-8 strings of length >= 3
        let mut i = 0;
        let mut strings = Vec::new();
        while i < data.len() {
            if data[i].is_ascii_graphic() {
                let start = i;
                while i < data.len() && (data[i].is_ascii_graphic() || data[i] == b' ') {
                    i += 1;
                }
                let len = i - start;
                if len >= 3 {
                    if let Ok(s) = std::str::from_utf8(&data[start..i]) {
                        strings.push(s.to_string());
                    }
                }
            } else {
                i += 1;
            }
        }

        println!("  Found {} string tokens:", strings.len());
        for s in strings.iter().take(20) {
            println!("    \"{}\"", s);
        }
    }

    Ok(())
}
