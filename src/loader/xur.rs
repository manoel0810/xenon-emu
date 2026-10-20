use byteorder::{BigEndian, ByteOrder};
use std::fs;
use std::path::Path;

/// Visual UI Node representing an element in Microsoft's XUI visual tree (.xur / XUIB)
#[derive(Debug, Clone)]
pub struct XuiVisualNode {
    pub id: String,
    pub class_name: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub text: Option<String>,
    pub children: Vec<XuiVisualNode>,
}

/// Parsed XUI Resource Scene (.xur)
#[derive(Debug, Clone)]
pub struct XurScene {
    pub name: String,
    pub version: u32,
    pub root: XuiVisualNode,
    pub classes: Vec<String>,
    pub raw_strings: Vec<String>,
}

impl XurScene {
    /// Parses an authentic compiled Xbox 360 UI Resource (.xur / XUIB)
    pub fn parse(name: &str, bytes: &[u8]) -> Result<Self, String> {
        // Find XUIB magic in file (might be inside XUIZ package or standalone)
        let mut xuib_off = None;
        for i in 0..bytes.len().saturating_sub(4) {
            if &bytes[i..i + 4] == b"XUIB" {
                xuib_off = Some(i);
                break;
            }
        }

        let off = xuib_off.ok_or_else(|| "Missing 'XUIB' header signature".to_string())?;
        let xuib = &bytes[off..];

        if xuib.len() < 12 {
            return Err("XUIB payload too small".into());
        }

        let version = BigEndian::read_u32(&xuib[4..8]);

        let mut classes = Vec::new();
        let mut raw_strings = Vec::new();
        let mut positions: Vec<(f32, f32, f32, f32)> = Vec::new();

        // Parse Chunks
        let mut cur = 12;
        while cur + 8 <= xuib.len() {
            let tag = &xuib[cur..cur + 4];
            if tag.iter().all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit()) {
                let chunk_size = BigEndian::read_u32(&xuib[cur + 4..cur + 8]) as usize;
                let chunk_data_start = cur + 8;
                let chunk_data_end = (chunk_data_start + chunk_size).min(xuib.len());

                if chunk_data_start <= xuib.len() {
                    let chunk_data = &xuib[chunk_data_start..chunk_data_end];
                    let tag_name = std::str::from_utf8(tag).unwrap_or("????");

                    match tag_name {
                        "STRN" => {
                            // Extract class names & control labels
                            let u16_chars: Vec<u16> = chunk_data
                                .chunks_exact(2)
                                .map(|c| BigEndian::read_u16(c))
                                .collect();
                            let text = String::from_utf16_lossy(&u16_chars);
                            for s in text.split('\0') {
                                let t = s.trim();
                                if !t.is_empty() {
                                    raw_strings.push(t.to_string());
                                    if t.starts_with("Xui") {
                                        classes.push(t.to_string());
                                    }
                                }
                            }
                        }
                        "VECT" => {
                            // Vectors: read (x, y, w, h) floating point values
                            for fchunk in chunk_data.chunks_exact(16) {
                                let x = BigEndian::read_f32(&fchunk[0..4]);
                                let y = BigEndian::read_f32(&fchunk[4..8]);
                                let z = BigEndian::read_f32(&fchunk[8..12]);
                                let w = BigEndian::read_f32(&fchunk[12..16]);
                                if x.is_finite() && y.is_finite() && z.is_finite() && w.is_finite() {
                                    positions.push((x, y, z, w));
                                }
                            }
                        }
                        _ => {}
                    }
                }

                cur += 8 + chunk_size;
            } else {
                cur += 1;
            }
        }

        // Build root visual node
        let root_class = classes.first().cloned().unwrap_or_else(|| "XuiCanvas".to_string());
        let (rx, ry, rw, rh) = positions.first().copied().unwrap_or((0.0, 0.0, 1280.0, 720.0));

        let mut children = Vec::new();
        for (i, cls) in classes.iter().skip(1).enumerate() {
            let (cx, cy, cw, ch) = positions.get(i + 1).copied().unwrap_or((
                120.0 + (i as f32 * 50.0),
                180.0 + (i as f32 * 60.0),
                300.0,
                50.0,
            ));

            children.push(XuiVisualNode {
                id: format!("{}_{}", cls, i + 1),
                class_name: cls.clone(),
                x: cx,
                y: cy,
                width: cw,
                height: ch,
                text: raw_strings.get(i).cloned(),
                children: Vec::new(),
            });
        }

        let root = XuiVisualNode {
            id: name.to_string(),
            class_name: root_class,
            x: rx,
            y: ry,
            width: rw,
            height: rh,
            text: None,
            children,
        };

        Ok(Self {
            name: name.to_string(),
            version,
            root,
            classes,
            raw_strings,
        })
    }

    /// Loads and parses an .xur file from disk
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let name = path
            .as_ref()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("scene");
        let bytes = fs::read(path.as_ref())
            .map_err(|e| format!("Failed to read .xur file '{}': {:?}", path.as_ref().display(), e))?;
        Self::parse(name, &bytes)
    }
}
