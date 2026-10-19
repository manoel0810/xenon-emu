use byteorder::{BigEndian, ByteOrder};
use cbc::cipher::{BlockModeDecrypt, KeyIvInit};
use std::collections::HashMap;
use crate::memory::MemoryBus;

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
    pub import_table: Vec<u32>,
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
    pub window_size: u32,
    pub first_block_size: u32,
    pub first_block_hash: [u8; 20],
    pub import_libraries: Vec<ImportLibrary>,
    pub pe_sections: Vec<PeSectionHeader>,
}

impl XexFile {
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < 0x24 {
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

        // Parse Optional Headers (according to xex2_info.h):
        // 0x00010100: Entry Point
        let entry_point = optional_headers.get(&0x00010100).copied().unwrap_or(0);

        // 0x00010201: Image Base Address
        let image_base = optional_headers.get(&0x00010201).copied().unwrap_or(load_address);

        // 0x000183FF: Original PE Name
        let mut original_pe_name = None;
        if let Some(&name_off) = optional_headers.get(&0x000183FF) {
            let n_off = name_off as usize;
            if n_off < data.len() {
                let end = data[n_off..].iter().position(|&b| b == 0).unwrap_or(0);
                if let Ok(s) = std::str::from_utf8(&data[n_off..n_off + end]) {
                    original_pe_name = Some(s.to_string());
                }
            }
        }

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
        let mut window_size = 0;
        let mut first_block_size = 0;
        let mut first_block_hash = [0u8; 20];

        if let Some(&fmt_off) = optional_headers.get(&0x000003FF) {
            let f_off = fmt_off as usize;
            if f_off + 12 <= data.len() {
                encryption_type = BigEndian::read_u16(&data[f_off + 4..f_off + 6]);
                compression_type = BigEndian::read_u16(&data[f_off + 6..f_off + 8]);
                window_size = BigEndian::read_u32(&data[f_off + 8..f_off + 12]);
                if f_off + 16 <= data.len() {
                    first_block_size = BigEndian::read_u32(&data[f_off + 12..f_off + 16]);
                }
                if f_off + 36 <= data.len() {
                    first_block_hash.copy_from_slice(&data[f_off + 16..f_off + 36]);
                }
            }
        }

        // Parse Import Libraries (0x000103FF)
        let mut import_libraries = Vec::new();
        if let Some(&imp_off) = optional_headers.get(&0x000103FF) {
            let i_off = imp_off as usize;
            if i_off + 12 <= data.len() {
                let total_size = BigEndian::read_u32(&data[i_off..i_off + 4]) as usize;
                let str_tbl_size = BigEndian::read_u32(&data[i_off + 4..i_off + 8]) as usize;
                let str_tbl_count = BigEndian::read_u32(&data[i_off + 8..i_off + 12]) as usize;

                let mut string_table = Vec::new();
                let mut str_pos = i_off + 12;
                let str_end = i_off + 12 + str_tbl_size;

                while str_pos < str_end && string_table.len() < str_tbl_count {
                    let mut end = str_pos;
                    while end < str_end && data[end] != 0 {
                        end += 1;
                    }
                    if let Ok(s) = std::str::from_utf8(&data[str_pos..end]) {
                        string_table.push(s.to_string());
                    } else {
                        string_table.push("unknown".to_string());
                    }
                    let mut advance = (end - str_pos) + 1;
                    if (advance % 4) != 0 {
                        advance += 4 - (advance % 4);
                    }
                    str_pos += advance;
                }

                let mut lib_off = i_off + 12 + str_tbl_size;
                let total_end = i_off + total_size;

                while lib_off + 40 <= total_end && lib_off + 40 <= data.len() {
                    let lib_size = BigEndian::read_u32(&data[lib_off..lib_off + 4]) as usize;
                    if lib_size == 0 {
                        break;
                    }

                    let version = BigEndian::read_u32(&data[lib_off + 28..lib_off + 32]);
                    let min_version = BigEndian::read_u32(&data[lib_off + 32..lib_off + 36]);
                    let name_index = BigEndian::read_u16(&data[lib_off + 36..lib_off + 38]) as usize;
                    let count = BigEndian::read_u16(&data[lib_off + 38..lib_off + 40]) as usize;

                    let name = string_table.get(name_index).cloned().unwrap_or_else(|| "unknown".into());

                    let mut import_table = Vec::with_capacity(count);
                    let table_start = lib_off + 40;
                    for k in 0..count {
                        if table_start + (k + 1) * 4 <= data.len() {
                            let rec_addr = BigEndian::read_u32(&data[table_start + k * 4..table_start + (k + 1) * 4]);
                            import_table.push(rec_addr);
                        }
                    }

                    import_libraries.push(ImportLibrary {
                        name,
                        version,
                        min_version,
                        import_table,
                    });

                    lib_off += lib_size;
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
            window_size,
            first_block_size,
            first_block_hash,
            import_libraries,
            pe_sections: Vec::new(),
        })
    }

    /// Decrypts the executable payload (from pe_offset) using AES-128-CBC with the session key
    pub fn decrypt_image_payload(&self, raw_xex_data: &[u8]) -> Result<Vec<u8>, String> {
        let pe_off = self.pe_offset as usize;
        if pe_off >= raw_xex_data.len() {
            return Err("PE offset out of bounds".into());
        }

        let mut payload = raw_xex_data[pe_off..].to_vec();
        let len = (payload.len() / 16) * 16;
        let payload = &mut payload[..len];

        if self.encryption_type == 1 {
            let iv = [0u8; 16];
            let mut cipher = Aes128CbcDec::new(&self.session_key.into(), &iv.into());
            cipher.decrypt_blocks(unsafe {
                std::slice::from_raw_parts_mut(
                    payload.as_mut_ptr() as *mut cbc::cipher::Block<Aes128CbcDec>,
                    len / 16,
                )
            });
        }

        Ok(payload.to_vec())
    }

    /// De-blocks the decrypted payload and verifies each block's SHA-1 hash.
    /// Returns the list of extracted LZX chunks.
    pub fn extract_lzx_chunks(&self, decrypted_payload: &[u8]) -> Result<Vec<Vec<u8>>, String> {
        if self.compression_type != 2 {
            return Err(format!("Not LZX compressed (compression type: {})", self.compression_type));
        }

        let mut chunks = Vec::new();
        let mut cur_block_size = self.first_block_size as usize;
        let mut cur_block_hash = self.first_block_hash;

        let mut offset = 0;
        let mut block_idx = 0;

        while cur_block_size > 0 {
            if offset + cur_block_size > decrypted_payload.len() {
                return Err(format!(
                    "Block {} exceeds payload bounds (offset: 0x{:X}, size: 0x{:X}, total len: 0x{:X})",
                    block_idx, offset, cur_block_size, decrypted_payload.len()
                ));
            }

            let block_bytes = &decrypted_payload[offset..offset + cur_block_size];

            // Verify block SHA-1 hash
            use sha1::{Digest, Sha1};
            let mut hasher = Sha1::new();
            hasher.update(block_bytes);
            let calculated_hash: [u8; 20] = hasher.finalize().into();

            println!("  [Block {:02}] Offset: 0x{:08X} | Size: {:5} bytes | Hash: {:02X}{:02X}{:02X}{:02X}...",
                block_idx, offset, cur_block_size, cur_block_hash[0], cur_block_hash[1], cur_block_hash[2], cur_block_hash[3]
            );

            if calculated_hash != cur_block_hash {
                return Err(format!(
                    "Block {} SHA-1 hash mismatch! Expected: {:02X?}, Got: {:02X?}",
                    block_idx, cur_block_hash, calculated_hash
                ));
            }

            if cur_block_size < 24 {
                return Err(format!("Block {} too small for header ({} bytes)", block_idx, cur_block_size));
            }

            let next_block_size = BigEndian::read_u32(&block_bytes[0..4]) as usize;
            let mut next_block_hash = [0u8; 20];
            next_block_hash.copy_from_slice(&block_bytes[4..24]);

            // Parse 16-bit chunk sizes and chunk data
            let mut chunk_pos = 24;
            while chunk_pos + 2 <= cur_block_size {
                let chunk_size = BigEndian::read_u16(&block_bytes[chunk_pos..chunk_pos + 2]) as usize;
                chunk_pos += 2;
                if chunk_size == 0 {
                    break;
                }
                if chunk_pos + chunk_size > cur_block_size {
                    return Err(format!(
                        "Block {} chunk overflow: pos {}, size {}, block_size {}",
                        block_idx, chunk_pos, chunk_size, cur_block_size
                    ));
                }
                chunks.push(block_bytes[chunk_pos..chunk_pos + chunk_size].to_vec());
                chunk_pos += chunk_size;
            }

            offset += cur_block_size;
            cur_block_size = next_block_size;
            cur_block_hash = next_block_hash;
            block_idx += 1;
        }

        println!(
            "[+] Validated {} XEX2 compressed blocks with SHA-1 hashes (extracted {} chunks)",
            block_idx, chunks.len()
        );

        Ok(chunks)
    }

    /// Decompresses all LZX chunks into the final uncompressed PE executable image.
    pub fn decompress_image(&self, decrypted_payload: &[u8]) -> Result<Vec<u8>, String> {
        let chunks = self.extract_lzx_chunks(decrypted_payload)?;

        // Map window size to lzxd::WindowSize
        let window_size = match self.window_size {
            0x8000 => lzxd::WindowSize::KB32,
            0x10000 => lzxd::WindowSize::KB64,
            0x20000 => lzxd::WindowSize::KB128,
            0x40000 => lzxd::WindowSize::KB256,
            0x80000 => lzxd::WindowSize::KB512,
            0x100000 => lzxd::WindowSize::MB1,
            0x200000 => lzxd::WindowSize::MB2,
            _ => lzxd::WindowSize::KB32,
        };

        let mut decoder = lzxd::Lzxd::new(window_size);
        let mut uncompressed_image = Vec::with_capacity(self.image_size as usize);
        let target_size = self.image_size as usize;

        for (idx, chunk) in chunks.iter().enumerate() {
            let remaining = target_size.saturating_sub(uncompressed_image.len());
            if remaining == 0 {
                break;
            }
            let output_len = std::cmp::min(remaining, 32 * 1024);

            match decoder.decompress_next(chunk, output_len) {
                Ok(decompressed) => {
                    uncompressed_image.extend_from_slice(decompressed);
                }
                Err(e) => {
                    return Err(format!(
                        "LZX decode error on chunk {}/{}: {:?} (decompressed so far: {} bytes)",
                        idx + 1, chunks.len(), e, uncompressed_image.len()
                    ));
                }
            }
        }

        // Pad to image_size if needed
        if uncompressed_image.len() < target_size {
            uncompressed_image.resize(target_size, 0);
        }

        println!(
            "[+] Successfully decompressed complete PE image: {} bytes ({} MB)!",
            uncompressed_image.len(), uncompressed_image.len() / (1024 * 1024)
        );

        Ok(uncompressed_image)
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
            "Compression:      {} (0x{:04X}) | Window: {} KB | First Block: {} bytes",
            match self.compression_type {
                0 => "Uncompressed",
                1 => "Basic Blocks",
                2 => "LZX Normal",
                3 => "Delta Patch",
                _ => "Custom",
            },
            self.compression_type,
            self.window_size / 1024,
            self.first_block_size
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
                    "  - {:<16} (Records: {:3})",
                    lib.name, lib.import_table.len()
                );
            }
        }

        println!("\nOptional Headers ({}):", self.optional_headers.len());
        for (k, v) in &self.optional_headers {
            println!("  0x{:08X} => 0x{:08X}", k, v);
        }
        println!("------------------------------------------------------------");
    }

    /// Rewrites all import thunks in memory with `sc 2` and `blr`,
    /// returning a mapping from thunk address to (library_name, ordinal).
    pub fn setup_import_thunks(
        &self,
        bus: &mut MemoryBus,
    ) -> Result<HashMap<u64, (String, u16)>, String> {
        let mut thunk_map = HashMap::new();
        let mut total_thunks = 0;
        let mut total_vars = 0;

        for lib in &self.import_libraries {
            for &record_addr in &lib.import_table {
                let addr = record_addr as u64;
                if let Ok(record_val) = bus.read_u32(addr) {
                    let record_type = ((record_val >> 24) & 0xFF) as u8;
                    let ordinal = (record_val & 0xFFFF) as u16;

                    if record_type == 1 {
                        // Function Thunk:
                        // Overwrite with:
                        //   sc 2  (0x44000042)
                        //   blr   (0x4E800020)
                        bus.write_u32(addr, 0x44000042)?;
                        bus.write_u32(addr + 4, 0x4E800020)?;
                        thunk_map.insert(addr, (lib.name.clone(), ordinal));
                        total_thunks += 1;
                    } else if record_type == 0 {
                        // Variable import
                        total_vars += 1;
                    }
                }
            }
        }

        println!(
            "[+] Setup Import Thunks: {} function thunks hooked with HLE traps, {} variables mapped across {} libraries.",
            total_thunks, total_vars, self.import_libraries.len()
        );

        Ok(thunk_map)
    }
}
