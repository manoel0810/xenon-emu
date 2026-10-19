use byteorder::{LittleEndian, ByteOrder};
use crate::memory::MemoryBus;

#[derive(Debug, Clone)]
pub struct PeSection {
    pub name: String,
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub raw_data_size: u32,
    pub raw_data_ptr: u32,
    pub characteristics: u32,
}

#[derive(Debug)]
pub struct PeImage {
    pub entry_point_rva: u32,
    pub image_base: u32,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub sections: Vec<PeSection>,
}

impl PeImage {
    pub fn parse(pe_bytes: &[u8]) -> Result<Self, String> {
        if pe_bytes.len() < 0x40 {
            return Err("PE image too small for DOS header".into());
        }

        // Verify DOS header ('MZ')
        if pe_bytes[0] != 0x4D || pe_bytes[1] != 0x5A {
            return Err("Invalid DOS magic (not 'MZ')".into());
        }

        // e_lfanew is at offset 0x3C (Little Endian in PE specification)
        let pe_offset = LittleEndian::read_u32(&pe_bytes[0x3C..0x40]) as usize;
        if pe_offset + 0x18 > pe_bytes.len() {
            return Err("PE header offset out of bounds".into());
        }

        // Verify PE signature ("PE\0\0")
        if &pe_bytes[pe_offset..pe_offset + 4] != b"PE\0\0" {
            return Err("Invalid PE signature".into());
        }

        let num_sections = LittleEndian::read_u16(&pe_bytes[pe_offset + 6..pe_offset + 8]);
        let opt_hdr_size = LittleEndian::read_u16(&pe_bytes[pe_offset + 20..pe_offset + 22]) as usize;

        let opt_hdr_off = pe_offset + 24;
        let entry_point_rva = LittleEndian::read_u32(&pe_bytes[opt_hdr_off + 16..opt_hdr_off + 20]);
        let image_base = LittleEndian::read_u32(&pe_bytes[opt_hdr_off + 28..opt_hdr_off + 32]);
        let section_alignment = LittleEndian::read_u32(&pe_bytes[opt_hdr_off + 32..opt_hdr_off + 36]);
        let file_alignment = LittleEndian::read_u32(&pe_bytes[opt_hdr_off + 36..opt_hdr_off + 40]);

        let sec_hdr_off = opt_hdr_off + opt_hdr_size;
        let mut sections = Vec::new();

        for i in 0..num_sections as usize {
            let s_off = sec_hdr_off + i * 40;
            if s_off + 40 > pe_bytes.len() {
                break;
            }

            let name_raw = &pe_bytes[s_off..s_off + 8];
            let name_end = name_raw.iter().position(|&b| b == 0).unwrap_or(8);
            let name = std::str::from_utf8(&name_raw[..name_end]).unwrap_or(".unknown").to_string();

            let virtual_size = LittleEndian::read_u32(&pe_bytes[s_off + 8..s_off + 12]);
            let virtual_address = LittleEndian::read_u32(&pe_bytes[s_off + 12..s_off + 16]);
            let raw_data_size = LittleEndian::read_u32(&pe_bytes[s_off + 16..s_off + 20]);
            let raw_data_ptr = LittleEndian::read_u32(&pe_bytes[s_off + 20..s_off + 24]);
            let characteristics = LittleEndian::read_u32(&pe_bytes[s_off + 36..s_off + 40]);

            sections.push(PeSection {
                name,
                virtual_size,
                virtual_address,
                raw_data_size,
                raw_data_ptr,
                characteristics,
            });
        }

        Ok(Self {
            entry_point_rva,
            image_base,
            section_alignment,
            file_alignment,
            sections,
        })
    }

    /// Loads the PE sections into memory bus at the specified base address
    pub fn load_into_memory(&self, pe_bytes: &[u8], base_vaddr: u64, bus: &mut MemoryBus) -> Result<(), String> {
        for sec in &self.sections {
            let sec_vaddr = base_vaddr + sec.virtual_address as u64;
            let raw_start = sec.raw_data_ptr as usize;
            let raw_end = raw_start + sec.raw_data_size as usize;

            if raw_start < pe_bytes.len() {
                let to_copy = std::cmp::min(raw_end, pe_bytes.len()) - raw_start;
                bus.load_bytes(sec_vaddr, &pe_bytes[raw_start..raw_start + to_copy])?;
                println!(
                    "  [PE SECTION] Loaded {:<10} at 0x{:016X} (VSize: 0x{:08X}, Raw: 0x{:08X})",
                    sec.name, sec_vaddr, sec.virtual_size, sec.raw_data_size
                );
            }
        }
        Ok(())
    }
}
