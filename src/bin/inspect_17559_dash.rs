use byteorder::ByteOrder;
use std::fs;
use xenon_emu::loader::XexFile;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("========================================================");
    println!("  Xenon-Emu: Inspecionando Dashboard 17559 Oficial      ");
    println!("========================================================");

    let dash_path = "../SystemUpdate_17559_USB/$SystemUpdate/dash.xex";
    let data = fs::read(dash_path)?;
    println!("[+] Carregados {} bytes de '{}'!", data.len(), dash_path);

    let xex = XexFile::parse(&data)?;
    xex.print_summary();

    println!("\n[*] Bibliotecas importadas pela Dashboard 17559:");
    for lib in &xex.import_libraries {
        println!("  - Library: '{}' ({} funcoes importadas)", lib.name, lib.import_table.len());
    }

    if let Some(&fmt_off) = xex.optional_headers.get(&0x000003FF) {
        let f_off = fmt_off as usize;
        let info_size = byteorder::BigEndian::read_u32(&data[f_off..f_off + 4]);
        let enc = byteorder::BigEndian::read_u16(&data[f_off + 4..f_off + 6]);
        let comp = byteorder::BigEndian::read_u16(&data[f_off + 6..f_off + 8]);
        println!("\n[+] Format Info at 0x{:X}: size={}, enc={}, comp={}", f_off, info_size, enc, comp);
        let block_count = (info_size as usize - 8) / 8;
        println!("  - Block count: {}", block_count);
        for b in 0..block_count {
            let b_off = f_off + 8 + b * 8;
            let d_sz = byteorder::BigEndian::read_u32(&data[b_off..b_off + 4]);
            let z_sz = byteorder::BigEndian::read_u32(&data[b_off + 4..b_off + 8]);
            println!("  - Block {}: data_size=0x{:X} ({}), zero_size=0x{:X} ({})", b, d_sz, d_sz, z_sz, z_sz);
        }
    }

    println!("\n[*] Tentando decodificar e descriptografar o payload da imagem...");
    match xex.decrypt_image_payload(&data) {
        Ok(decrypted) => {
            println!("[+] Payload descriptografado com sucesso! ({} bytes)", decrypted.len());
            match xex.decompress_image(&decrypted) {
                Ok(decompressed) => {
                    println!("[+] Imagem PE descomprimida com sucesso! ({} bytes / {:.2} MB)",
                        decompressed.len(), decompressed.len() as f64 / (1024.0 * 1024.0)
                    );
                    if decompressed.len() >= 0x40 {
                        let pe_hdr_off = byteorder::LittleEndian::read_u32(&decompressed[0x3C..0x40]) as usize;
                        println!("  - Offset do PE Header (e_lfanew): 0x{:X}", pe_hdr_off);
                        if pe_hdr_off + 24 <= decompressed.len() {
                            let pe_sig = &decompressed[pe_hdr_off..pe_hdr_off + 4];
                            let num_sections = byteorder::LittleEndian::read_u16(&decompressed[pe_hdr_off + 6..pe_hdr_off + 8]);
                            let opt_hdr_size = byteorder::LittleEndian::read_u16(&decompressed[pe_hdr_off + 20..pe_hdr_off + 22]) as usize;
                            let opt_hdr_off = pe_hdr_off + 24;
                            let entry_point_rva = byteorder::LittleEndian::read_u32(&decompressed[opt_hdr_off + 16..opt_hdr_off + 20]);
                            let image_base_pe = byteorder::LittleEndian::read_u32(&decompressed[opt_hdr_off + 28..opt_hdr_off + 32]);
                            println!("  - Assinatura PE: {:?} | Secoes: {} | Opt Header Size: 0x{:X}", std::str::from_utf8(pe_sig).unwrap_or("???"), num_sections, opt_hdr_size);
                            println!("  - PE EntryPoint RVA: 0x{:08X} (VAddr: 0x{:08X}), ImageBase: 0x{:08X}", entry_point_rva, image_base_pe + entry_point_rva, image_base_pe);
                            let ep_raw = entry_point_rva as usize;
                            println!("\n  [DISASM around entry point 0x{:08X}]:", ep_raw);
                            for a in (ep_raw.saturating_sub(0x30)..ep_raw + 0x40).step_by(4) {
                                if a + 4 <= decompressed.len() {
                                    let insn = byteorder::BigEndian::read_u32(&decompressed[a..a + 4]);
                                    let mark = if a == ep_raw { "==>" } else { "   " };
                                    println!("    {} [0x{:08X}] 0x{:08X}", mark, 0x92000000 + a as u64, insn);
                                }
                            }
                            if let Some(&name_off) = xex.optional_headers.get(&0x000183FF) {
                                let n_off = name_off as usize;
                                let name_len = byteorder::BigEndian::read_u32(&data[n_off..n_off + 4]) as usize;
                                let pe_name = std::str::from_utf8(&data[n_off + 4..n_off + 4 + name_len]).unwrap_or("?");
                                println!("  - Original PE Name from XEX: '{}'", pe_name);
                            }
                            if let Some(&tls_off) = xex.optional_headers.get(&0x00020104) {
                                let t_off = tls_off as usize;
                                let slot_count = byteorder::BigEndian::read_u32(&data[t_off..t_off + 4]);
                                let raw_addr = byteorder::BigEndian::read_u32(&data[t_off + 4..t_off + 8]);
                                let data_sz = byteorder::BigEndian::read_u32(&data[t_off + 8..t_off + 12]);
                                let raw_sz = byteorder::BigEndian::read_u32(&data[t_off + 12..t_off + 16]);
                                println!("  - TLS Info: slot_count={}, raw_addr=0x{:08X}, data_size=0x{:X}, raw_size=0x{:X}", slot_count, raw_addr, data_sz, raw_sz);
                            }
                            let sec_table_start = pe_hdr_off + 24 + opt_hdr_size;
                            for s in 0..num_sections as usize {
                                let s_off = sec_table_start + s * 40;
                                if s_off + 40 <= decompressed.len() {
                                    let mut name_bytes = [0u8; 8];
                                    name_bytes.copy_from_slice(&decompressed[s_off..s_off + 8]);
                                    let name = std::str::from_utf8(&name_bytes).unwrap_or("?").trim_matches('\0');
                                    let v_size = byteorder::LittleEndian::read_u32(&decompressed[s_off + 8..s_off + 12]);
                                    let v_addr = byteorder::LittleEndian::read_u32(&decompressed[s_off + 12..s_off + 16]);
                                    let r_size = byteorder::LittleEndian::read_u32(&decompressed[s_off + 16..s_off + 20]);
                                    let r_ptr = byteorder::LittleEndian::read_u32(&decompressed[s_off + 20..s_off + 24]);
                                    println!("    [Sec {:02}] {:<8} | VAddr: 0x{:08X} (size 0x{:08X}) | RawPtr: 0x{:08X} (size 0x{:08X})",
                                        s, name, v_addr, v_size, r_ptr, r_size);
                                }
                            }
                        }
                    }

                    // Scan and parse embedded XUIZ / XZP packages
                    let mut xzips = Vec::new();
                    for i in 0..decompressed.len().saturating_sub(4) {
                        if &decompressed[i..i + 4] == b"XUIZ" {
                            xzips.push(i);
                        }
                    }
                    for (pkg_idx, &offset) in xzips.iter().enumerate() {
                        let remaining_slice = &decompressed[offset..];
                        if let Ok(pkg) = xenon_emu::loader::XzpPackage::parse(remaining_slice) {
                            let first_files: Vec<&str> = pkg.entries.iter().take(4).map(|e| e.name.as_str()).collect();
                            println!("  [Pacote {:02} @ 0x{:08X}] Tamanho: {:6} B | Arquivos ({:3}): {:?}",
                                pkg_idx, offset, pkg.total_size, pkg.entries.len(), first_files
                            );
                        }
                    }
                }
                Err(e) => println!("  [!] Erro ao descomprimir imagem: {}", e),
            }
        }
        Err(e) => println!("  [!] Erro ao descriptografar payload: {}", e),
    }

    Ok(())
}
