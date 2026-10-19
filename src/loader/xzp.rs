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

            // Filenames can be UTF-16 BE or ASCII
            let name_bytes_len = fnlen * 2;
            if bufpos + name_bytes_len > bytes.len() {
                break;
            }

            let name_raw = &bytes[bufpos..bufpos + name_bytes_len];
            bufpos += name_bytes_len;

            // Decode UTF-16 BE
            let u16_chars: Vec<u16> = name_raw
                .chunks_exact(2)
                .map(|c| BigEndian::read_u16(c))
                .collect();
            let name = String::from_utf16_lossy(&u16_chars)
                .trim_matches(char::from(0))
                .to_string();

            // Extract payload
            let file_start = fptr as usize;
            let file_end = file_start + flen as usize;
            let data = if file_end <= bytes.len() && file_start < bytes.len() {
                bytes[file_start..file_end].to_vec()
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
