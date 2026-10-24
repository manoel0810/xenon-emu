use byteorder::{BigEndian, ByteOrder};

#[derive(Debug, Clone)]
pub struct XzpEntry {
    pub name: String,
    pub file_size: u32,
    pub file_offset: u32,
    pub data: Vec<u8>,
}

#[derive(Debug)]
pub struct XzpPackage {
    pub flags: u32,
    pub total_size: u32,
    pub data_ptr: u32,
    pub entries: Vec<XzpEntry>,
}

impl XzpPackage {
    /// Parses an in-memory XUIZ (.xzp) archive container
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 0x16 {
            return Err("Data too small for XUIZ header".into());
        }

        if &bytes[0..4] != b"XUIZ" {
            return Err(format!("Invalid XUIZ signature: {:02X?}", &bytes[0..4]));
        }

        let flags = BigEndian::read_u32(&bytes[4..8]);
        let total_size = BigEndian::read_u32(&bytes[8..12]);
        let _reserved = BigEndian::read_u32(&bytes[12..16]);
        let data_ptr = BigEndian::read_u32(&bytes[16..20]);
        let num_entries = BigEndian::read_u16(&bytes[20..22]) as usize;

        let mut entries = Vec::new();
        let mut bufpos = 22usize;

        for _ in 0..num_entries {
            if bufpos + 9 > bytes.len() {
                break;
            }

            let flen = BigEndian::read_u32(&bytes[bufpos..bufpos + 4]);
            let fptr = BigEndian::read_u32(&bytes[bufpos + 4..bufpos + 8]);
            let fnlen = bytes[bufpos + 8] as usize;
            bufpos += 9;

            let (name, advance) = if bufpos + fnlen <= bytes.len() {
                if let Ok(s) = std::str::from_utf8(&bytes[bufpos..bufpos + fnlen]) {
                    if s.chars().all(|c| c.is_ascii_graphic() || c == ' ' || c == '\0') {
                        (s.trim_matches('\0').to_string(), fnlen)
                    } else if bufpos + fnlen * 2 <= bytes.len() {
                        let u16_chars: Vec<u16> = bytes[bufpos..bufpos + fnlen * 2]
                            .chunks_exact(2)
                            .map(|c| BigEndian::read_u16(c))
                            .collect();
                        (String::from_utf16_lossy(&u16_chars).trim_matches('\0').to_string(), fnlen * 2)
                    } else {
                        (s.trim_matches('\0').to_string(), fnlen)
                    }
                } else if bufpos + fnlen * 2 <= bytes.len() {
                    let u16_chars: Vec<u16> = bytes[bufpos..bufpos + fnlen * 2]
                        .chunks_exact(2)
                        .map(|c| BigEndian::read_u16(c))
                        .collect();
                    (String::from_utf16_lossy(&u16_chars).trim_matches('\0').to_string(), fnlen * 2)
                } else {
                    ("<unknown>".to_string(), fnlen)
                }
            } else {
                break;
            };

            bufpos += advance;

            // Extract payload relative to data_ptr
            let file_start = data_ptr as usize + fptr as usize;
            let file_end = file_start + flen as usize;
            let data = if file_end <= bytes.len() && file_start < bytes.len() {
                bytes[file_start..file_end].to_vec()
            } else if (fptr as usize + flen as usize) <= bytes.len() {
                bytes[fptr as usize..fptr as usize + flen as usize].to_vec()
            } else {
                Vec::new()
            };

            entries.push(XzpEntry {
                name,
                file_size: flen,
                file_offset: fptr,
                data,
            });
        }

        Ok(Self {
            flags,
            total_size,
            data_ptr,
            entries,
        })
    }
}
