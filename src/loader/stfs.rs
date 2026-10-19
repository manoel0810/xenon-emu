use byteorder::{BigEndian, ByteOrder};

#[derive(Debug, Clone)]
pub struct StfsFileEntry {
    pub name: String,
    pub flags: u8,
    pub block_count: u32,
    pub start_block: u32,
    pub file_size: u32,
    pub path_indicator: u16,
}

pub struct StfsPackage {
    pub magic: [u8; 4],
    pub entries: Vec<StfsFileEntry>,
}

impl StfsPackage {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 0xD000 {
            return Err("File too small for STFS package".into());
        }

        let magic = [data[0], data[1], data[2], data[3]];
        if &magic != b"PIRS" && &magic != b"LIVE" && &magic != b"CON " {
            return Err(format!("Invalid STFS magic: {:?}", magic));
        }

        let mut entries = Vec::new();
        let ft_offset = 0xC000;
        let ft_size = 0x1000; // 1 block = 4096 bytes

        for i in (0..ft_size).step_by(64) {
            let entry_raw = &data[ft_offset + i..ft_offset + i + 64];
            if entry_raw[0] == 0 {
                continue;
            }

            let flags = entry_raw[0x28];
            let name_len = (flags & 0x3F) as usize;
            let name = match std::str::from_utf8(&entry_raw[..name_len.min(40)]) {
                Ok(s) => s.to_string(),
                Err(_) => continue,
            };

            // Start block at 0x2F..0x32 (3 bytes Little Endian)
            let start_block = (entry_raw[0x2F] as u32)
                | ((entry_raw[0x30] as u32) << 8)
                | ((entry_raw[0x31] as u32) << 16);

            // Block count at 0x29..0x2C (3 bytes Little Endian)
            let block_count = (entry_raw[0x29] as u32)
                | ((entry_raw[0x2A] as u32) << 8)
                | ((entry_raw[0x2B] as u32) << 16);

            // Path indicator at 0x32..0x34
            let path_indicator = BigEndian::read_u16(&entry_raw[0x32..0x34]);

            // File size at 0x34..0x38 (4 bytes Big Endian)
            let file_size = BigEndian::read_u32(&entry_raw[0x34..0x38]);

            entries.push(StfsFileEntry {
                name,
                flags,
                block_count,
                start_block,
                file_size,
                path_indicator,
            });
        }

        Ok(Self { magic, entries })
    }

    /// Maps an STFS logical data block to its physical file byte offset,
    /// correctly skipping interspersed L0 (every 170 blocks) and L1 (every 28900 blocks) hash tables.
    pub fn block_to_offset(block: u32) -> usize {
        let mut block_adjust = 0;
        if block >= 0xAA {
            block_adjust += block / 0xAA;
        }
        if block > 0x70E4 {
            block_adjust += block / 0x70E4;
        }
        ((block + block_adjust) as usize * 4096) + 0xC000
    }

    /// Extracts a file by reading its blocks
    pub fn extract_file(&self, package_bytes: &[u8], name: &str) -> Result<Vec<u8>, String> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("File '{}' not found in STFS package", name))?;

        let mut data = Vec::with_capacity(entry.file_size as usize);
        let target_size = entry.file_size as usize;

        for b in entry.start_block..(entry.start_block + entry.block_count) {
            let offset = Self::block_to_offset(b);
            if offset >= package_bytes.len() {
                break;
            }

            let to_read = (target_size - data.len()).min(4096);
            if offset + to_read > package_bytes.len() {
                let available = package_bytes.len() - offset;
                data.extend_from_slice(&package_bytes[offset..offset + available.min(to_read)]);
                break;
            } else {
                data.extend_from_slice(&package_bytes[offset..offset + to_read]);
            }

            if data.len() >= target_size {
                break;
            }
        }

        Ok(data)
    }
}
