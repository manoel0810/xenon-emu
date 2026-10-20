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

#[derive(Debug, Clone)]
pub struct ExportSymbol {
    pub ordinal: u32,
    pub name: Option<String>,
    pub rva: u32,
}

#[derive(Debug)]
pub struct PeImage {
    pub entry_point_rva: u32,
    pub image_base: u32,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub sections: Vec<PeSection>,
    pub exports: Vec<ExportSymbol>,
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

        // Check Data Directory 0 (Export Directory)
        let mut exports = Vec::new();
        let export_rva = if opt_hdr_size >= 104 {
            LittleEndian::read_u32(&pe_bytes[opt_hdr_off + 96..opt_hdr_off + 100])
        } else {
            0
        };

        let export_target_rva = if export_rva > 0 {
            export_rva
        } else {
            // Fallback: look for .edata section
            sections.iter().find(|s| s.name.eq_ignore_ascii_case(".edata")).map(|s| s.virtual_address).unwrap_or(0)
        };

        if export_target_rva > 0 {
            // Find section containing export_target_rva
            if let Some(sec) = sections.iter().find(|s| export_target_rva >= s.virtual_address && export_target_rva < s.virtual_address + s.virtual_size) {
                let off = (sec.raw_data_ptr + (export_target_rva - sec.virtual_address)) as usize;
                if off + 40 <= pe_bytes.len() {
                    let base_ordinal = LittleEndian::read_u32(&pe_bytes[off + 16..off + 20]);
                    let num_functions = LittleEndian::read_u32(&pe_bytes[off + 20..off + 24]);
                    let num_names = LittleEndian::read_u32(&pe_bytes[off + 24..off + 28]);
                    let funcs_rva = LittleEndian::read_u32(&pe_bytes[off + 28..off + 32]);
                    let names_rva = LittleEndian::read_u32(&pe_bytes[off + 32..off + 36]);
                    let ords_rva = LittleEndian::read_u32(&pe_bytes[off + 36..off + 40]);

                    // Helper to translate RVA (either absolute or relative to export dir)
                    let resolve_rva = |target: u32| -> Option<usize> {
                        // First try standard RVA
                        for s in &sections {
                            if target >= s.virtual_address && target < s.virtual_address + s.virtual_size {
                                return Some((s.raw_data_ptr + (target - s.virtual_address)) as usize);
                            }
                        }
                        // Second try relative to export dir
                        let rel = export_target_rva + target;
                        for s in &sections {
                            if rel >= s.virtual_address && rel < s.virtual_address + s.virtual_size {
                                return Some((s.raw_data_ptr + (rel - s.virtual_address)) as usize);
                            }
                        }
                        None
                    };
                    // Pre-index names by function index in a single O(num_names) pass
                    let mut name_map = std::collections::HashMap::new();
                    if num_names > 0 {
                        if let (Some(names_table_off), Some(ords_table_off)) = (resolve_rva(names_rva), resolve_rva(ords_rva)) {
                            for n_idx in 0..num_names.min(4096) as usize {
                                let ord_entry_off = ords_table_off + n_idx * 2;
                                let name_ptr_off = names_table_off + n_idx * 4;
                                if ord_entry_off + 2 <= pe_bytes.len() && name_ptr_off + 4 <= pe_bytes.len() {
                                    let fn_idx = LittleEndian::read_u16(&pe_bytes[ord_entry_off..ord_entry_off + 2]) as u32;
                                    let name_str_rva = LittleEndian::read_u32(&pe_bytes[name_ptr_off..name_ptr_off + 4]);
                                    if let Some(str_off) = resolve_rva(name_str_rva) {
                                        let end = pe_bytes[str_off..].iter().position(|&b| b == 0).unwrap_or(0);
                                        if let Ok(s) = std::str::from_utf8(&pe_bytes[str_off..str_off + end]) {
                                            name_map.insert(fn_idx, s.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }

                    let max_funcs = num_functions.min(1024);
                    for i in 0..max_funcs {
                        let ord = base_ordinal + i;
                        let fn_rva_off = resolve_rva(funcs_rva).map(|o| o + (i as usize) * 4);
                        let fn_rva = fn_rva_off.and_then(|o| if o + 4 <= pe_bytes.len() { Some(LittleEndian::read_u32(&pe_bytes[o..o+4])) } else { None }).unwrap_or(0);
                        let sym_name = name_map.remove(&i);

                        exports.push(ExportSymbol {
                            ordinal: ord,
                            name: sym_name,
                            rva: fn_rva,
                        });
                    }
                }
            }
        }

        Ok(Self {
            entry_point_rva,
            image_base,
            section_alignment,
            file_alignment,
            sections,
            exports,
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
