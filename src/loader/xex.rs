use byteorder::{BigEndian, ByteOrder};
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use std::collections::HashMap;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

pub const XEX2_RETAIL_KEY: [u8; 16] = [
    0x20, 0xB1, 0x85, 0xA5, 0x9D, 0x28, 0xFD, 0xC3,
    0x40, 0x58, 0x3F, 0xBB, 0x08, 0x96, 0xBF, 0x91,
];

pub const XEX2_DEVKIT_KEY: [u8; 16] = [0u8; 16];

#[derive(Debug, Clone)]
pub struct ExecutionInfo {
    pub media_id: u32,
    pub version: u32,
    pub base_version: u32,
    pub title_id: u32,
    pub platform: u8,
    pub executable_table: u8,
    pub disc_number: u8,
    pub disc_count: u8,
}

#[derive(Debug, Clone)]
pub struct ImportLibrary {
    pub name: String,
    pub version: u32,
    pub min_version: u32,
    pub record_count: u16,
    pub import_table_addr: u32,
}

#[derive(Debug, Clone)]
pub struct PeSectionHeader {
    pub name: String,
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub raw_data_size: u32,
    pub raw_data_ptr: u32,
    pub characteristics: u32,
}

pub struct XexFile {
    pub magic: [u8; 4],
    pub module_flags: u32,
    pub pe_offset: u32,
    pub security_offset: u32,
    pub optional_headers: HashMap<u32, u32>,
    pub original_pe_name: Option<String>,
    pub execution_info: Option<ExecutionInfo>,
    pub entry_point: u32,
    pub image_base: u32,
    pub load_address: u32,
    pub image_size: u32,
    pub session_key: [u8; 16],
    pub encryption_type: u16,
    pub compression_type: u16,
    pub import_libraries: Vec<ImportLibrary>,
    pub pe_sections: Vec<PeSectionHeader>,
}

impl XexFile {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 0x20 {
            return Err("File too small for XEX2 header".into());
        }

        let magic = [data[0], data[1], data[2], data[3]];
        if &magic != b"XEX2" && &magic != b"XEX1" {
            return Err(format!("Invalid XEX magic: {:?}", magic));
        }

        let module_flags = BigEndian::read_u32(&data[4..8]);
        let pe_offset = BigEndian::read_u32(&data[8..12]);
        let _reserved = BigEndian::read_u32(&data[12..16]);
        let security_offset = BigEndian::read_u32(&data[16..20]);
        let optional_header_count = BigEndian::read_u32(&data[20..24]);

        let mut optional_headers = HashMap::new();
        let mut pos = 24;
        for _ in 0..optional_header_count {
            if pos + 8 > data.len() {
                break;
            }
            let key = BigEndian::read_u32(&data[pos..pos + 4]);
            let val = BigEndian::read_u32(&data[pos + 4..pos + 8]);
            optional_headers.insert(key, val);
            pos += 8;
        }

        // Parse Security Info
        let sec_off = security_offset as usize;
        if sec_off + 0x160 > data.len() {
            return Err("Security offset exceeds file bounds".into());
        }
        let _header_size = BigEndian::read_u32(&data[sec_off..sec_off + 4]);
        let image_size = BigEndian::read_u32(&data[sec_off + 4..sec_off + 8]);
        let load_address = BigEndian::read_u32(&data[sec_off + 0x110..sec_off + 0x114]);
        let encrypted_aes_key = &data[sec_off + 0x150..sec_off + 0x160];

        // Decrypt Session Key using Retail Key
        let mut session_key = [0u8; 16];
        session_key.copy_from_slice(encrypted_aes_key);
        let iv = [0u8; 16];
        let mut cipher = Aes128CbcDec::new(&XEX2_RETAIL_KEY.into(), &iv.into());
        cipher.decrypt_block((&mut session_key).into());

        // Parse Optional Headers:
        // 0x00010100: Original PE Name
        let mut original_pe_name = None;
        if let Some(&name_off) = optional_headers.get(&0x00010100) {
            let n_off = name_off as usize;
            if n_off < data.len() {
                let end = data[n_off..].iter().position(|&b| b == 0).unwrap_or(0);
                if let Ok(s) = std::str::from_utf8(&data[n_off..n_off + end]) {
                    original_pe_name = Some(s.to_string());
                }
            }
        }

        // 0x000183FF: Entry Point (Direct value or offset)
        let entry_point = optional_headers.get(&0x000183FF).copied().unwrap_or(0);

        // 0x000200FF: Image Base Address
        let image_base = optional_headers.get(&0x000200FF).copied().unwrap_or(load_address);

        // 0x00040006: Execution Info
        let mut execution_info = None;
        if let Some(&exec_off) = optional_headers.get(&0x00040006) {
            let e_off = exec_off as usize;
            if e_off + 24 <= data.len() {
                execution_info = Some(ExecutionInfo {
                    media_id: BigEndian::read_u32(&data[e_off..e_off + 4]),
                    version: BigEndian::read_u32(&data[e_off + 4..e_off + 8]),
                    base_version: BigEndian::read_u32(&data[e_off + 8..e_off + 12]),
                    title_id: BigEndian::read_u32(&data[e_off + 12..e_off + 16]),
                    platform: data[e_off + 16],
                    executable_table: data[e_off + 17],
                    disc_number: data[e_off + 18],
                    disc_count: data[e_off + 19],
                });
            }
        }

        // 0x000003FF: File Format Info (Encryption and Compression types)
        let mut encryption_type = 0;
        let mut compression_type = 0;
        if let Some(&fmt_off) = optional_headers.get(&0x000003FF) {
            let f_off = fmt_off as usize;
            if f_off + 4 <= data.len() {
                encryption_type = BigEndian::read_u16(&data[f_off..f_off + 2]);
                compression_type = BigEndian::read_u16(&data[f_off + 2..f_off + 4]);
            }
        }

        // Parse Import Libraries (0x00020104)
        let mut import_libraries = Vec::new();
        if let Some(&imp_off) = optional_headers.get(&0x00020104) {
            let mut i_off = imp_off as usize;
            if i_off + 8 <= data.len() {
                let count = BigEndian::read_u32(&data[i_off + 4..i_off + 8]) as usize;
                i_off += 8;
                for _ in 0..count {
                    if i_off + 36 > data.len() {
                        break;
                    }
                    // String table + import records
                    let name_len = data[i_off..i_off + 8].iter().position(|&b| b == 0).unwrap_or(8);
                    let name = std::str::from_utf8(&data[i_off..i_off + name_len])
                        .unwrap_or("unknown")
                        .to_string();
                    let version = BigEndian::read_u32(&data[i_off + 8..i_off + 12]);
                    let min_version = BigEndian::read_u32(&data[i_off + 12..i_off + 16]);
                    let record_count = BigEndian::read_u16(&data[i_off + 18..i_off + 20]);
                    let import_table_addr = BigEndian::read_u32(&data[i_off + 20..i_off + 24]);

                    import_libraries.push(ImportLibrary {
                        name,
                        version,
                        min_version,
                        record_count,
                        import_table_addr,
                    });
                    i_off += 36;
                }
            }
        }

        Ok(Self {
            magic,
            module_flags,
            pe_offset,
            security_offset,
            optional_headers,
            original_pe_name,
            execution_info,
            entry_point,
            image_base,
            load_address,
            image_size,
            session_key,
            encryption_type,
            compression_type,
            import_libraries,
            pe_sections: Vec::new(),
        })
    }

    pub fn print_summary(&self) {
        println!("------------------------------------------------------------");
        println!("             Xbox 360 Executable (XEX2) Details             ");
        println!("------------------------------------------------------------");
        println!("Magic:            {:?}", std::str::from_utf8(&self.magic).unwrap_or("????"));
        println!("Original PE Name: {}", self.original_pe_name.as_deref().unwrap_or("<unknown>"));
        println!("Load Address:     0x{:08X}", self.load_address);
        println!("Image Base:       0x{:08X}", self.image_base);
        println!("Entry Point:      0x{:08X}", self.entry_point);
        println!("Image Size:       0x{:08X} ({} MB)", self.image_size, self.image_size / (1024 * 1024));
        println!("PE Offset:        0x{:08X}", self.pe_offset);
        println!("Security Offset:  0x{:08X}", self.security_offset);
        println!("Decrypted Key:    {:02X?}", self.session_key);
        println!(
            "Encryption:       {} (0x{:04X})",
            if self.encryption_type == 1 { "AES-128 Normal" } else { "None" },
            self.encryption_type
        );
        println!(
            "Compression:      {} (0x{:04X})",
            match self.compression_type {
                0 => "Uncompressed",
                1 => "Basic Blocks",
                2 => "LZX Normal",
                3 => "Delta Patch",
                _ => "Custom",
            },
            self.compression_type
        );

        if let Some(exec) = &self.execution_info {
            let major = (exec.version >> 28) & 0xF;
            let minor = (exec.version >> 24) & 0xF;
            let build = (exec.version >> 8) & 0xFFFF;
            let qfe = exec.version & 0xFF;
            println!("Title ID:         {:08X}", exec.title_id);
            println!("Version:          {}.{}.{}.{}", major, minor, build, qfe);
        }

        if !self.import_libraries.is_empty() {
            println!("\nImport Libraries ({}):", self.import_libraries.len());
            for lib in &self.import_libraries {
                println!(
                    "  - {:<16} (Records: {:3}, Import Table: 0x{:08X})",
                    lib.name, lib.record_count, lib.import_table_addr
                );
            }
        }
        println!("------------------------------------------------------------");
    }
}
