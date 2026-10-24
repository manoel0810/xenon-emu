//! Minimal PowerPC disassembler for the 17559 dash image (diagnostics only).
//! Usage: cargo run --release --bin ppcdis -- <start_hex> <end_hex> [more ranges as pairs...]
//! Resolves `bl` targets that are kernel import thunks to their xboxkrnl/xam export names.
use byteorder::{BigEndian, ByteOrder};
use std::fs;
use xenon_emu::kernel::exports::{get_xam_name, get_xboxkrnl_name};
use xenon_emu::loader::XexFile;
use xenon_emu::memory::bus::MemoryBus;

fn sx(v: u32, bits: u32) -> i64 {
    let s = 32 - bits;
    (((v << s) as i32) >> s) as i64
}

fn disasm(pc: u64, w: u32, thunks: &std::collections::HashMap<u64, (String, u16)>) -> String {
    let op = w >> 26;
    let rt = (w >> 21) & 31;
    let ra = (w >> 16) & 31;
    let rb = (w >> 11) & 31;
    let d = sx(w & 0xFFFF, 16);
    let name_of = |t: u64| -> String {
        if let Some((lib, ord)) = thunks.get(&t) {
            let n = if lib.to_ascii_lowercase().contains("xam") { get_xam_name(*ord) } else { get_xboxkrnl_name(*ord) };
            format!("{} <{}!{}>", format_args!("0x{:08X}", t), lib, n.map(|s| s.to_string()).unwrap_or(format!("#{}", ord)))
        } else {
            format!("0x{:08X}", t)
        }
    };
    match op {
        7 => format!("mulli r{rt},r{ra},{d}"),
        8 => format!("subfic r{rt},r{ra},{d}"),
        10 => format!("cmplwi cr{},r{ra},0x{:X}", (w >> 23) & 7, w & 0xFFFF),
        11 => format!("cmpwi cr{},r{ra},{d}", (w >> 23) & 7),
        12 => format!("addic r{rt},r{ra},{d}"),
        13 => format!("addic. r{rt},r{ra},{d}"),
        14 => if ra == 0 { format!("li r{rt},{d}") } else { format!("addi r{rt},r{ra},{d}") },
        15 => if ra == 0 { format!("lis r{rt},0x{:X}", w & 0xFFFF) } else { format!("addis r{rt},r{ra},0x{:X}", w & 0xFFFF) },
        16 => {
            let bo = rt; let bi = ra;
            let tgt = if w & 2 != 0 { (w & 0xFFFC) as u64 } else { (pc as i64 + sx(w & 0xFFFC, 16)) as u64 };
            format!("bc{} {bo},{bi},{}", if w & 1 != 0 { "l" } else { "" }, name_of(tgt))
        }
        17 => "sc".to_string(),
        18 => {
            let li = sx(w & 0x03FF_FFFC, 26);
            let tgt = if w & 2 != 0 { li as u64 } else { (pc as i64 + li) as u64 };
            format!("{} {}", if w & 1 != 0 { "bl" } else { "b" }, name_of(tgt & 0xFFFF_FFFF))
        }
        19 => match (w >> 1) & 0x3FF {
            16 => if rt == 20 { "blr".into() } else { format!("bclr {rt},{ra}") },
            528 => if w & 1 != 0 { "bctrl".into() } else { "bctr".into() },
            _ => format!(".long 0x{:08X} (op19)", w),
        },
        20 => format!("rlwimi r{ra},r{rt},{},{},{}", rb, (w >> 6) & 31, (w >> 1) & 31),
        21 => format!("rlwinm r{ra},r{rt},{},{},{}", rb, (w >> 6) & 31, (w >> 1) & 31),
        24 => if w == 0x6000_0000 { "nop".into() } else { format!("ori r{ra},r{rt},0x{:X}", w & 0xFFFF) },
        25 => format!("oris r{ra},r{rt},0x{:X}", w & 0xFFFF),
        26 => format!("xori r{ra},r{rt},0x{:X}", w & 0xFFFF),
        28 => format!("andi. r{ra},r{rt},0x{:X}", w & 0xFFFF),
        29 => format!("andis. r{ra},r{rt},0x{:X}", w & 0xFFFF),
        30 => format!(".long 0x{:08X} (op30 rld*)", w),
        31 => {
            let xo = (w >> 1) & 0x3FF;
            match xo {
                0 => format!("cmpw cr{},r{ra},r{rb}", (w >> 23) & 7),
                32 => format!("cmplw cr{},r{ra},r{rb}", (w >> 23) & 7),
                266 => format!("add r{rt},r{ra},r{rb}"),
                40 => format!("subf r{rt},r{ra},r{rb}"),
                444 => if rt == rb { format!("mr r{ra},r{rt}") } else { format!("or r{ra},r{rt},r{rb}") },
                28 => format!("and r{ra},r{rt},r{rb}"),
                316 => format!("xor r{ra},r{rt},r{rb}"),
                339 => format!("mfspr r{rt},{}", ((w >> 16) & 31) | (((w >> 11) & 31) << 5)),
                467 => match ((w >> 16) & 31) | (((w >> 11) & 31) << 5) { 8 => format!("mtlr r{rt}"), 9 => format!("mtctr r{rt}"), n => format!("mtspr {n},r{rt}") },
                23 => format!("lwzx r{rt},r{ra},r{rb}"),
                151 => format!("stwx r{rt},r{ra},r{rb}"),
                87 => format!("lbzx r{rt},r{ra},r{rb}"),
                215 => format!("stbx r{rt},r{ra},r{rb}"),
                279 => format!("lhzx r{rt},r{ra},r{rb}"),
                24 => format!("slw r{ra},r{rt},r{rb}"),
                536 => format!("srw r{ra},r{rt},r{rb}"),
                _ => format!(".long 0x{:08X} (op31 xo{})", w, xo),
            }
        }
        32 => format!("lwz r{rt},{d}(r{ra})"),
        33 => format!("lwzu r{rt},{d}(r{ra})"),
        34 => format!("lbz r{rt},{d}(r{ra})"),
        36 => format!("stw r{rt},{d}(r{ra})"),
        37 => format!("stwu r{rt},{d}(r{ra})"),
        38 => format!("stb r{rt},{d}(r{ra})"),
        40 => format!("lhz r{rt},{d}(r{ra})"),
        42 => format!("lha r{rt},{d}(r{ra})"),
        44 => format!("sth r{rt},{d}(r{ra})"),
        48 => format!("lfs f{rt},{d}(r{ra})"),
        52 => format!("stfs f{rt},{d}(r{ra})"),
        58 => format!("ld r{rt},{}(r{ra})", sx(w & 0xFFFC, 16)),
        62 => format!("std r{rt},{}(r{ra})", sx(w & 0xFFFC, 16)),
        4 | 5 | 6 => format!(".long 0x{:08X} (vmx op{})", w, op),
        _ => format!(".long 0x{:08X}", w),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `str <addr>...`: print big-endian UTF-16 strings at the given image addresses
    let str_mode = args.first().map(|s| s == "str").unwrap_or(false);
    if str_mode { args.remove(0); }
    if !str_mode && args.first().map(|s| s != "refs" && s != "words" && s != "callers" && s != "importcallers" && s != "lis").unwrap_or(true) && (args.len() < 2 || args.len() % 2 != 0) {
        eprintln!("usage: ppcdis <start_hex> <end_hex> [<start> <end> ...]");
        return Ok(());
    }
    let dash_bytes = fs::read("../SystemUpdate_17559_USB/$SystemUpdate/dash.xex")?;
    let dash_xex = XexFile::parse(&dash_bytes)?;
    let decrypted = dash_xex.decrypt_image_payload(&dash_bytes)?;
    let img = dash_xex.decompress_image(&decrypted)?;
    let base = dash_xex.image_base as u64;
    let mut bus = MemoryBus::new();
    for (i, b) in img.chunks(4).enumerate() {
        if b.len() == 4 {
            let w = BigEndian::read_u32(b);
            if w != 0 { let _ = bus.write_u32(base + (i as u64) * 4, w); }
        }
    }
    let thunks = dash_xex.setup_import_thunks(&mut bus)?;
    let thunks: std::collections::HashMap<u64, (String, u16)> =
        thunks.into_iter().map(|(k, v)| (k & 0xFFFF_FFFF, (v.0, v.1 as u16))).collect();
    // `refs <imm16_hex> <hi16_hex>`: list `addi`s with that immediate (and data words hi16:imm16) in the image
    if args.first().map(|s| s == "refs").unwrap_or(false) {
        let imm = u32::from_str_radix(args[1].trim_start_matches("0x"), 16)?;
        let hi = u32::from_str_radix(args[2].trim_start_matches("0x"), 16)?;
        for (i, b) in img.chunks(4).enumerate() {
            if b.len() < 4 { break; }
            let w = BigEndian::read_u32(b);
            let a = base + (i as u64) * 4;
            if (w >> 26) == 14 && (w & 0xFFFF) == imm { println!("addi  0x{:08X}: {:08X}", a, w); }
            if w == ((hi << 16) | imm) { println!("data  0x{:08X}: {:08X}", a, w); }
        }
        return Ok(());
    }
    // `refs <imm16_hex> <hi16_hex>`: list `addi`s with that immediate (and data words hi16:imm16) in the image
    if args.first().map(|s| s == "refs").unwrap_or(false) {
        let imm = u32::from_str_radix(args[1].trim_start_matches("0x"), 16)?;
        let hi = u32::from_str_radix(args[2].trim_start_matches("0x"), 16)?;
        for (i, b) in img.chunks(4).enumerate() {
            if b.len() < 4 { break; }
            let w = BigEndian::read_u32(b);
            let a = base + (i as u64) * 4;
            if (w >> 26) == 14 && (w & 0xFFFF) == imm { println!("addi  0x{:08X}: {:08X}", a, w); }
            if w == ((hi << 16) | imm) { println!("data  0x{:08X}: {:08X}", a, w); }
        }
        return Ok(());
    }
    // `words <addr> <count>`: dump big-endian words from the image
    // `callers <target>`: list every `bl`/`b` that targets the address, plus data words holding it (vtables / pointers)
    if args.first().map(|s| s == "callers").unwrap_or(false) {
        let target = u64::from_str_radix(args[1].trim_start_matches("0x"), 16)?;
        for (i, b) in img.chunks(4).enumerate() {
            if b.len() < 4 { break; }
            let w = BigEndian::read_u32(b);
            let a = base + (i as u64) * 4;
            if (w >> 26) == 18 && (w & 2) == 0 {
                let li = (((w & 0x03FF_FFFC) << 6) as i32 >> 6) as i64;
                if (a as i64 + li) as u64 == target { println!("{}  0x{:08X}", if w & 1 != 0 { "bl  " } else { "b   " }, a); }
            }
            if w as u64 == target { println!("data 0x{:08X}", a); }
        }
        return Ok(());
    }
    // `importcallers <ExportName>`: list every `bl` whose target thunk resolves to that kernel/xam export name
    if args.first().map(|s| s == "importcallers").unwrap_or(false) {
        let want = args[1].to_ascii_lowercase();
        for (i, b) in img.chunks(4).enumerate() {
            if b.len() < 4 { break; }
            let w = BigEndian::read_u32(b);
            let a = base + (i as u64) * 4;
            if (w >> 26) == 18 && (w & 2) == 0 {
                let li = (((w & 0x03FF_FFFC) << 6) as i32 >> 6) as i64;
                let tgt = ((a as i64 + li) as u64) & 0xFFFF_FFFF;
                if let Some((lib, ord)) = thunks.get(&tgt) {
                    let n = if lib.to_ascii_lowercase().contains("xam") { get_xam_name(*ord) } else { get_xboxkrnl_name(*ord) };
                    if n.map(|s| s.to_ascii_lowercase() == want).unwrap_or(false) { println!("{}  0x{:08X} -> {}!{}", if w & 1 != 0 { "bl" } else { "b " }, a, lib, n.unwrap()); }
                }
            }
        }
        return Ok(());
    }
    // `lis <imm16_hex>`: list every `lis rX,imm` (addis with RA=0) with that immediate
    if args.first().map(|s| s == "lis").unwrap_or(false) {
        let imm = u32::from_str_radix(args[1].trim_start_matches("0x"), 16)?;
        for (i, b) in img.chunks(4).enumerate() {
            if b.len() < 4 { break; }
            let w = BigEndian::read_u32(b);
            if (w >> 26) == 15 && ((w >> 16) & 31) == 0 && (w & 0xFFFF) == imm { println!("lis  0x{:08X}: {:08X}  rD=r{}", base + (i as u64) * 4, w, (w >> 21) & 31); }
        }
        return Ok(());
    }
    if args.first().map(|s| s == "words").unwrap_or(false) {
        let addr = u64::from_str_radix(args[1].trim_start_matches("0x"), 16)?;
        let cnt: usize = args[2].parse()?;
        for i in 0..cnt {
            if i % 8 == 0 { print!("\n{:08X}:", addr + (i as u64) * 4); }
            let off = (addr - base) as usize + i * 4;
            print!(" {:08X}", BigEndian::read_u32(&img[off..off + 4]));
        }
        println!();
        return Ok(());
    }
    if str_mode {
        for a in &args {
            let addr = u64::from_str_radix(a.trim_start_matches("0x").trim_start_matches("0X"), 16)?;
            let mut off = (addr - base) as usize;
            let mut s = String::new();
            while off + 2 <= img.len() {
                let c = u16::from_be_bytes([img[off], img[off + 1]]);
                if c == 0 || s.len() > 200 { break; }
                s.push(char::from_u32(c as u32).unwrap_or('?'));
                off += 2;
            }
            // Also show an ASCII reading in case it is not UTF-16
            let o0 = (addr - base) as usize;
            let ascii: String = img[o0..(o0 + 48).min(img.len())].iter().take_while(|&&b| b != 0).map(|&b| if (32..127).contains(&b) { b as char } else { '.' }).collect();
            println!("0x{:08X}: utf16=\"{}\" ascii=\"{}\"", addr, s, ascii);
        }
        return Ok(());
    }
    for pair in args.chunks(2) {
        let p = |s: &str| u64::from_str_radix(s.trim_start_matches("0x").trim_start_matches("0X"), 16);
        let (start, end) = (p(&pair[0])?, p(&pair[1])?);
        println!("--- 0x{:08X}..0x{:08X}", start, end);
        let mut a = start;
        while a < end {
            let off = (a - base) as usize;
            if off + 4 > img.len() { break; }
            let w = BigEndian::read_u32(&img[off..off + 4]);
            println!("0x{:08X}: {:08X}  {}", a, w, disasm(a, w, &thunks));
            a += 4;
        }
    }
    Ok(())
}
