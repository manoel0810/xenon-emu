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

        // Extract genuine UTF-16 length-prefixed strings
        let raw_strings = extract_xuib_strings(xuib);
        let mut classes = Vec::new();
        for s in &raw_strings {
            if s.starts_with("Xui") {
                classes.push(s.clone());
            }
        }

        let mut positions: Vec<(f32, f32, f32, f32)> = Vec::new();

        // Scan for VECT chunk if present
        if let Some(pos) = xuib.windows(4).position(|w| w == b"VECT") {
            if pos + 8 <= xuib.len() {
                let chunk_size = BigEndian::read_u32(&xuib[pos + 4..pos + 8]) as usize;
                let vstart = pos + 8;
                let vend = (vstart + chunk_size).min(xuib.len());
                let vdata = &xuib[vstart..vend];
                for fchunk in vdata.chunks_exact(16) {
                    let x = BigEndian::read_f32(&fchunk[0..4]);
                    let y = BigEndian::read_f32(&fchunk[4..8]);
                    let z = BigEndian::read_f32(&fchunk[8..12]);
                    let w = BigEndian::read_f32(&fchunk[12..16]);
                    if x.is_finite() && y.is_finite() && z.is_finite() && w.is_finite() {
                        positions.push((x, y, z, w));
                    }
                }
            }
        }

        // Build root visual node
        let root_class = classes.first().cloned().unwrap_or_else(|| "XuiCanvas".to_string());
        let (rx, ry, rw, rh) = positions.first().copied().unwrap_or((0.0, 0.0, 1280.0, 720.0));

        let mut children = Vec::new();
        // Create nodes for distinct visual elements and controls
        for (i, elem_name) in raw_strings.iter().enumerate() {
            if elem_name.starts_with("Xui") && elem_name != "XuiCanvas" {
                continue;
            }
            let (cx, cy, cw, ch) = positions.get(i).copied().unwrap_or((
                140.0 + ((i % 4) as f32 * 260.0),
                180.0 + ((i / 4) as f32 * 75.0),
                240.0,
                55.0,
            ));

            children.push(XuiVisualNode {
                id: elem_name.clone(),
                class_name: "XuiControl".to_string(),
                x: cx,
                y: cy,
                width: cw,
                height: ch,
                text: Some(elem_name.clone()),
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
