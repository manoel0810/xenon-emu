use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;
use image::{RgbaImage, Rgba};

pub fn xg_untile_variant(x: u32, y: u32, w: u32, mode: usize) -> u32 {
    let aligned_width = (w + 31) & !31;
    let log_bpp = 2u32; // 32-bpp = 4 bytes
    let macro_val = ((x >> 5) + (y >> 5) * (aligned_width >> 5)) << (log_bpp + 7);
    let micro = ((x & 7) + ((y & 6) << 2)) << log_bpp;
    
    let y1_term = if mode == 1 { 0 } else { (y & 1) << 4 };
    let y8_term = (y & 8) << (3 + log_bpp);

    let offset = macro_val
        + ((micro & !15) << 1)
        + (micro & 15)
        + y8_term
        + y1_term;

    let bank = match mode {
        0 => ((((y & 8) >> 2) + (x >> 3)) & 3) << 6,
        1 => 0,
        2 => ((((y & 8) >> 3) ^ (x >> 3)) & 3) << 6,
        3 => (((y >> 3) + (x >> 3)) & 3) << 6,
        4 => ((((y & 8) >> 2) ^ (x >> 3)) & 3) << 6,
        _ => ((((y & 8) >> 2) + (x >> 3)) & 3) << 6,
    };

    let result = (((offset & !511) << 3)
        + ((offset & 448) << 2)
        + (offset & 63)
        + ((y & 16) << 7)
        + bank) >> log_bpp;
    result
}

pub fn tiled_offset_2d_xenia(x: u32, y: u32, pitch_h: u32, pitch_v: u32, log2_bpp: u32) -> u32 {
    let z = 0;
    let base = ((z >> 2) * (pitch_v >> 4) + (y >> 4)) * (pitch_h >> 5);
    let mut micro = ((z >> 2) + (y >> 3)) & 1;
    micro += (((micro << 1) + (x >> 3)) & 3) << 1;
    let mut macro_v = ((((y & 6) << 2) + (x & 7)) << (log2_bpp + 6)) >> 6;
    macro_v = ((((((base + (x >> 5)) << (log2_bpp + 6)) & 0xFFFFFFF) << 1) + (macro_v & !15)) << 1) 
        + (macro_v & 15) 
        + ((z & 3) << (log2_bpp + 6)) 
        + ((y & 1) << 4);
    let res = ((((((((macro_v >> 6) & 7) + ((micro & 1) << 3)) << 3) + (micro & !1)) << 2) + (macro_v & !511)) << 3) + (macro_v & 63);
    res >> log2_bpp
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let target_ip = "192.168.3.6";
    println!("========================================================");
    println!("  Capturando Framebuffer Real da Dashboard do Xbox 360  ");
    println!("  IP: {}:730 (XBDM)                                     ", target_ip);
    println!("========================================================");

    // If raw file exists locally, load it; otherwise connect and download
    let raw_bin_path = "xbox360_real_framebuffer.raw";
    let raw_data = if std::path::Path::new(raw_bin_path).exists() {
        println!("[*] Carregando dump bruto de '{}'...", raw_bin_path);
        let mut data = std::fs::read(raw_bin_path)?;
        if data.starts_with(b"pitch=") {
            if let Some(pos) = data.windows(2).position(|w| w == b"\r\n") {
                data.drain(..pos + 2);
            }
        }
        data
    } else {
        let stream = TcpStream::connect(format!("{}:730", target_ip))?;
        stream.set_read_timeout(Some(Duration::from_millis(5000)))?;
        stream.set_write_timeout(Some(Duration::from_millis(5000)))?;

        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if trimmed.starts_with("201 ") || (!trimmed.starts_with("201-") && !trimmed.is_empty()) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        reader.get_mut().write_all(b"screenshot\r\n")?;
        reader.get_mut().flush()?;

        let mut header_line = String::new();
        reader.read_line(&mut header_line)?;
        let mut fbsize: usize = 0x3C0000;
        for part in header_line.split_whitespace() {
            if let Some((k, v)) = part.split_once('=') {
                if k == "framebuffersize" {
                    let val_clean = v.trim_start_matches("0x").trim_start_matches("0X");
                    if let Ok(s) = usize::from_str_radix(val_clean, 16) {
                        fbsize = s;
                    }
                }
            }
        }
        let mut buf = vec![0u8; fbsize];
        reader.read_exact(&mut buf)?;
        std::fs::write(raw_bin_path, &buf)?;
        buf
    };

    let width: u32 = 1280;
    let height: u32 = 720;
    let pitch: usize = 5120;
    println!("[+] Processando {} bytes do Framebuffer ({}x{})...", raw_data.len(), width, height);

    let max_pixels = raw_data.len() / 4;
    // Render variants 0 to 4
    for mode in 0..=4 {
        let mut img = RgbaImage::new(width, height);
        for y in 0..height {
            for x in 0..width {
                let tiled_idx = xg_untile_variant(x, y, width, mode) as usize;
                if tiled_idx < max_pixels {
                    let off = tiled_idx * 4;
                    let b0 = raw_data[off];
                    let b1 = raw_data[off + 1];
                    let b2 = raw_data[off + 2];
                    img.put_pixel(x, y, Rgba([b2, b1, b0, 255]));
                }
            }
        }
        let p = format!("../xbox360_real_var_{}.png", mode);
        img.save(&p)?;
        println!("    -> Modo {} salvo em '{}'", mode, p);
    }

    // Check sample pixel values in center of screen
    let cy = height as usize / 2;
    let cx = width as usize / 2;
    let center_off = cy * pitch + cx * 4;
    if center_off + 3 < raw_data.len() {
        println!("\n[*] Amostra de pixel central ({}, {}):", cx, cy);
        println!("    Bytes brutos: {:02X} {:02X} {:02X} {:02X}", 
            raw_data[center_off], raw_data[center_off+1], raw_data[center_off+2], raw_data[center_off+3]
        );
    }

    println!("\n========================================================");
    println!("  [+] Framebuffer Real da Dashboard Processado com Exito!");
    println!("========================================================");

    Ok(())
}
