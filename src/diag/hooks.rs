//! XENON_PC_HOOK: register/memory dumps when a guest thread reaches one of the configured PCs.

use super::DiagConfig;
use crate::cpu::Registers;
use crate::memory::MemoryBus;

fn read_utf16_be(bus: &MemoryBus, p: u64) -> String {
    let mut s = String::new();
    let mut a = p & 0xFFFF_FFFF;
    for _ in 0..64 {
        match bus.read_u16(a) { Ok(0) | Err(_) => break, Ok(c) => s.push(char::from_u32(c as u32).unwrap_or('?')) }
        a += 2;
    }
    s
}

/// Prints one PC hook hit (the `XENON_PC_HOOK_*` extras first, then the register line).
pub fn dump(cfg: &DiagConfig, bus: &MemoryBus, r: &Registers, hit: u64, frame: u64, thread_name: &str, pc: u64) {
    if let Some(spec) = &cfg.pc_hook_mem {
        // "<reg>:<hex offset>:<word count>" e.g. "31:360:40" dumps 40 words at r31+0x360
        let p: Vec<&str> = spec.split(':').collect();
        if p.len() == 3 {
            if let (Ok(rn), Ok(off), Ok(cnt)) = (p[0].parse::<usize>(), u64::from_str_radix(p[1], 16), p[2].parse::<u64>()) {
                let base = (r.gpr[rn] & 0xFFFF_FFFF).wrapping_add(off);
                let mut line = format!("    [MEM r{}+0x{:X} = 0x{:08X}]", rn, off, base);
                for i in 0..cnt {
                    if i % 8 == 0 { line.push_str(&format!("\n      {:08X}:", base + i * 4)); }
                    line.push_str(&format!(" {:08X}", bus.read_u32(base + i * 4).unwrap_or(0xDEADBEEF)));
                }
                println!("{}", line);
            }
        }
    }
    if let Some(spec) = &cfg.pc_hook_list {
        // "<reg>": walk a circular doubly linked list (flink at +0) starting at the address held in GPR <reg>
        if let Ok(rn) = spec.trim().parse::<usize>() {
            let head = r.gpr[rn] & 0xFFFF_FFFF;
            let mut cur = bus.read_u32(head).unwrap_or(0) as u64;
            let mut out = format!("head=0x{:08X} ->", head);
            for _ in 0..24 {
                let hdr = cur.wrapping_sub(8) & 0xFFFF_FFFF;
                let size_units = bus.read_u16(hdr).unwrap_or(0);
                out.push_str(&format!(" 0x{:08X}(sz={:#x},fl={:02X})", cur, size_units, bus.read_u8(hdr + 5).unwrap_or(0)));
                if cur == head || cur == 0 { break; }
                cur = bus.read_u32(cur).unwrap_or(0) as u64;
            }
            println!("    [LIST] {}", out);
        }
    }
    if let Some(regs) = &cfg.pc_hook_strregs {
        // e.g. "4,3": print the big-endian UTF-16 string each listed GPR points at
        for rn in regs.split(',').filter_map(|x| x.trim().parse::<usize>().ok()) {
            println!("    [STRREG] r{}=0x{:08X} -> \"{}\"", rn, r.gpr[rn] & 0xFFFF_FFFF, read_utf16_be(bus, r.gpr[rn]));
        }
    }
    if cfg.pc_hook_r3str {
        // r3 points at a struct whose +4 / +8 members are pointers to big-endian UTF-16 strings
        let base3 = r.gpr[3] & 0xFFFF_FFFF;
        let p4 = bus.read_u32(base3 + 4).unwrap_or(0) as u64;
        let p8 = bus.read_u32(base3 + 8).unwrap_or(0) as u64;
        println!("    [R3STR] name=\"{}\" base=\"{}\"", read_utf16_be(bus, p4), read_utf16_be(bus, p8));
    }
    println!("  [PC_HOOK #{} frame {} {}] pc=0x{:08X} lr=0x{:08X} r1=0x{:08X} r3=0x{:X} r4=0x{:X} r5=0x{:X} r6=0x{:X} r7=0x{:X} r8=0x{:X} r9=0x{:X} r10=0x{:X} r11=0x{:X} r29=0x{:X} r30=0x{:X} r31=0x{:X} ctr=0x{:X} callers:{}",
        hit, frame, thread_name.replace(" ", "_"), pc & 0xFFFF_FFFF, r.lr & 0xFFFF_FFFF, r.gpr[1] & 0xFFFF_FFFF, r.gpr[3], r.gpr[4], r.gpr[5], r.gpr[6], r.gpr[7], r.gpr[8], r.gpr[9], r.gpr[10], r.gpr[11], r.gpr[29], r.gpr[30], r.gpr[31], r.ctr,
        bus.backchain(r.gpr[1], 14));
}
