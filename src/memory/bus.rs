use byteorder::{BigEndian, ByteOrder};

/// Xbox 360 Physical Memory Bus
pub struct MemoryBus {
    /// 512 MB Main GDDR3 / DDR3 RAM
    pub ram: Vec<u8>,
    /// 512 KB Internal CPU SRAM / L2 Cache (used as Cache-As-RAM during 1BL and CB stages)
    pub sram: Vec<u8>,
    /// Optional watchpoint address (low 32 bits physical)
    pub watch_addr: Option<u64>,
    /// Last executed PC (set by CPU/main for diagnostic tracing)
    pub current_pc: u64,
    /// Link register / stack pointer of the running thread (diagnostics only)
    pub current_lr: u64,
    pub current_sp: u64,
    pub current_thread: String,
    /// GPU register file written through the MMIO window 0x7FC80000..0x7FC8FFFF (byte offset -> last value).
    /// The Xenos command processor reads CP_RB_WPTR (offset 0x714) from here instead of guessing it from ring contents.
    pub gpu_mmio: std::collections::HashMap<u32, u32>,
    /// Count of guest stores into the GPU MMIO window (diagnostics)
    pub gpu_mmio_writes: u64,
    /// Number of times the write watchpoint has matched (diagnostics)
    pub watch_hits: u64,
    /// Sparse backing for the guest virtual-alloc window (0x70000000..0x8FFFFFFF).
    /// Key = 64 KB chunk index inside the window, value = physical RAM offset of the chunk.
    window_map: Vec<u32>,
    window_pool_next: usize,
    /// Guest time base shared by all threads (mftb), advanced by Cpu::step.
    pub timebase: crate::cpu::timebase::TimeBase,
}

/// Guest virtual-alloc window (0x40000000..0x8FFFFFFF) handed out by NtAllocateVirtualMemory. These addresses used to be
/// masked with 0x1FFFFFFF, aliasing them onto the same physical RAM as the 0x9xxxxxxx
/// physical-allocation / GPU buffers and the dash image. They now get private backing pages.
const WINDOW_START: u64 = 0x4000_0000;
const WINDOW_END: u64 = 0x9000_0000;
const WINDOW_CHUNK: u64 = 0x1_0000; // 64 KB
const WINDOW_POOL_START: usize = 0x1400_0000;
const WINDOW_POOL_END: usize = 0x2000_0000;
static ZERO_PAGE: [u8; 4096] = [0u8; 4096];

/// GPU register window mapped by the kernel at 0x7FC80000 (CP_RB_WPTR = +0x714 ...). It must NOT be backed by plain RAM.
pub const GPU_MMIO_BASE: u64 = 0x7FC8_0000;
pub const GPU_MMIO_SIZE: u64 = 0x1_0000;
/// Byte offset of CP_RB_WPTR inside the window
pub const GPU_MMIO_CP_RB_WPTR: u32 = 0x714;

#[inline(always)]
/// GPU registers whose MMIO reads return fixed values (Xenia Canary 451f9155, GraphicsSystem::ReadRegister in
/// src/xenia/gpu/graphics_system.cc). Other registers read back the last value written. `reg` = offset / 4.
pub fn gpu_mmio_fixed_read(reg: u32) -> Option<u32> {
    match reg {
        0x0F00 => Some(0x0810_0748), // RB_EDRAM_TIMING
        0x0F01 => Some(0x0000_200E), // RB_BC_CONTROL
        0x0F2D => Some(0x00BB_BBBB), // RB_HSIO_INTERFACE_ALIGNER_VALUE
        0x1951 => Some(1),           // interrupt status: vblank (the dash's interrupt callback checks bit 0)
        0x1961 => Some(0x0500_02D0), // AVIVO_D1MODE_VIEWPORT_SIZE: 1280x720
        _ => None,
    }
}

fn mmio_offset(addr: u64) -> Option<u32> {
    let a = addr & 0xFFFF_FFFF;
    if a >= GPU_MMIO_BASE && a < GPU_MMIO_BASE + GPU_MMIO_SIZE { Some((a - GPU_MMIO_BASE) as u32) } else { None }
}

#[inline(always)]
fn window_addr(addr: u64) -> Option<u64> {
    let hi = addr >> 32;
    if hi != 0 && hi != 0xFFFF_FFFF { return None; }
    let a = addr & 0xFFFF_FFFF;
    if a >= WINDOW_START && a < WINDOW_END { Some(a) } else { None }
}

const RAM_SIZE: usize = 512 * 1024 * 1024; // 512 MB
const SRAM_SIZE: usize = 512 * 1024;       // 512 KB
const SRAM_PREFIX: u64 = 0x8000_0200_0000_0000;

impl MemoryBus {
    pub fn new() -> Self {
        let watch_addr = crate::diag::config().watch_addr;
        Self {
            ram: vec![0u8; RAM_SIZE],
            sram: vec![0u8; SRAM_SIZE],
            watch_addr,
            current_pc: 0,
            current_lr: 0,
            current_sp: 0,
            current_thread: String::new(),
            gpu_mmio: std::collections::HashMap::new(),
            gpu_mmio_writes: 0,
            watch_hits: 0,
            window_map: vec![u32::MAX; ((WINDOW_END - WINDOW_START) / WINDOW_CHUNK) as usize],
            window_pool_next: WINDOW_POOL_START,
            timebase: Default::default(),
        }
    }

    #[inline(always)]
    fn check_watch(&mut self, addr: u64, size: usize, val_hex: u64) {
        if let Some(target) = self.watch_addr {
            let a = addr & 0x1FFF_FFFF;
            let t = target & 0x1FFF_FFFF;
            if a <= t + 16 && a + size as u64 >= t {
                self.watch_hits += 1;
                println!(
                    "  [WATCHPOINT WRITE] PC: 0x{:08X} -> wrote {} bytes to 0x{:08X} (phys 0x{:08X}) = 0x{:X}",
                    self.current_pc, size, addr, a, val_hex
                );
                // Walk the Xbox 360 backchain: [sp] = caller's sp, [caller_sp - 8] = saved LR
                let mut chain = String::new();
                let mut sp = self.current_sp & 0xFFFF_FFFF;
                for _ in 0..6 {
                    let Some(next) = self.peek_u32(sp) else { break };
                    let next = next as u64;
                    if next == 0 { break; }
                    match self.peek_u32(next.wrapping_sub(8)) {
                        Some(lr) => chain.push_str(&format!(" 0x{:08X}", lr)),
                        None => break,
                    }
                    sp = next;
                }
                println!(
                    "      THREAD: {} | LR: 0x{:08X} | SP: 0x{:08X} | callers:{}",
                    self.current_thread, self.current_lr, self.current_sp, chain
                );
                if crate::diag::config().watch_mem {
                    // state BEFORE this write: 24 words starting 0x30 bytes below the watched address
                    let base = (target & 0xFFFF_FFFF).wrapping_sub(0x30) & !0xF;
                    let mut line = String::new();
                    for i in 0..24u64 {
                        if i % 4 == 0 { line.push_str(&format!("\n        {:08X}:", base + i * 4)); }
                        match self.peek_u32(base + i * 4) { Some(v) => line.push_str(&format!(" {:08X}", v)), None => line.push_str(" --------") }
                    }
                    println!("      MEM(before write):{}", line);
                }
            }
        }
    }

    /// Translates a 64-bit address into an internal buffer slice
    fn translate_address_mut(&mut self, addr: u64) -> Result<&mut [u8], String> {
        if (addr & 0xFFFF_FFFF_0000_0000) == SRAM_PREFIX {
            let offset = (addr & 0x0000_0000_FFFF_FFFF) as usize;
            if offset < SRAM_SIZE {
                return Ok(&mut self.sram[offset..]);
            } else {
                return Err(format!("SRAM access out of bounds: 0x{:016X}", addr));
            }
        }

        if let Some(a) = window_addr(addr) {
            let chunk = ((a - WINDOW_START) / WINDOW_CHUNK) as usize;
            let base = if self.window_map[chunk] != u32::MAX {
                self.window_map[chunk] as usize
            } else {
                let b = self.window_pool_next;
                if b + WINDOW_CHUNK as usize > WINDOW_POOL_END {
                    return Err(format!("Virtual-alloc window backing pool exhausted at 0x{:016X}", addr));
                }
                self.window_pool_next += WINDOW_CHUNK as usize;
                self.window_map[chunk] = b as u32;
                b
            };
            let off = base + ((a - WINDOW_START) % WINDOW_CHUNK) as usize;
            return Ok(&mut self.ram[off..]);
        }

        // Direct physical RAM access
        let ram_offset = (addr & 0x1FFF_FFFF) as usize;
        if ram_offset < RAM_SIZE {
            Ok(&mut self.ram[ram_offset..])
        } else {
            Err(format!("Physical RAM access out of bounds: 0x{:016X}", addr))
        }
    }

    fn translate_address(&self, addr: u64) -> Result<&[u8], String> {
        if (addr & 0xFFFF_FFFF_0000_0000) == SRAM_PREFIX {
            let offset = (addr & 0x0000_0000_FFFF_FFFF) as usize;
            if offset < SRAM_SIZE {
                return Ok(&self.sram[offset..]);
            } else {
                return Err(format!("SRAM access out of bounds: 0x{:016X}", addr));
            }
        }

        if let Some(a) = window_addr(addr) {
            let chunk = ((a - WINDOW_START) / WINDOW_CHUNK) as usize;
            return if self.window_map[chunk] != u32::MAX {
                Ok(&self.ram[self.window_map[chunk] as usize + ((a - WINDOW_START) % WINDOW_CHUNK) as usize..])
            } else {
                Ok(&ZERO_PAGE[..]) // never written: reads as zero
            };
        }

        let ram_offset = (addr & 0x1FFF_FFFF) as usize;
        if ram_offset < RAM_SIZE {
            Ok(&self.ram[ram_offset..])
        } else {
            Err(format!("Physical RAM access out of bounds: 0x{:016X}", addr))
        }
    }

    /// Walks the Xbox 360 backchain ([sp] = caller sp, [caller_sp - 8] = saved LR) for diagnostics.
    pub fn backchain(&self, sp: u64, depth: usize) -> String {
        let mut chain = String::new();
        let mut sp = sp & 0xFFFF_FFFF;
        for _ in 0..depth {
            let Some(next) = self.peek_u32(sp) else { break };
            let next = next as u64;
            if next == 0 { break; }
            match self.peek_u32(next.wrapping_sub(8)) {
                Some(lr) => chain.push_str(&format!(" 0x{:08X}", lr)),
                None => break,
            }
            sp = next;
        }
        chain
    }

    /// Non-failing 32-bit read used by diagnostics.
    fn peek_u32(&self, addr: u64) -> Option<u32> {
        let s = self.translate_address(addr).ok()?;
        if s.len() < 4 { return None; }
        Some(BigEndian::read_u32(s))
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, String> {
        let slice = self.translate_address(addr)?;
        Ok(slice[0])
    }

    pub fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), String> {
        self.check_watch(addr, 1, val as u64);
        let slice = self.translate_address_mut(addr)?;
        slice[0] = val;
        Ok(())
    }

    pub fn read_u16(&self, addr: u64) -> Result<u16, String> {
        let slice = self.translate_address(addr)?;
        if slice.len() < 2 {
            return Err("Short read".into());
        }
        Ok(BigEndian::read_u16(slice))
    }

    pub fn write_u16(&mut self, addr: u64, val: u16) -> Result<(), String> {
        self.check_watch(addr, 2, val as u64);
        let slice = self.translate_address_mut(addr)?;
        if slice.len() < 2 {
            return Err("Short write".into());
        }
        BigEndian::write_u16(slice, val);
        Ok(())
    }

    pub fn read_u32(&self, addr: u64) -> Result<u32, String> {
        if let Some(off) = mmio_offset(addr) {
            if let Some(v) = gpu_mmio_fixed_read(off / 4) {
                return Ok(v);
            }
            return Ok(self.gpu_mmio.get(&off).copied().unwrap_or(0));
        }
        let slice = self.translate_address(addr)?;
        if slice.len() < 4 {
            return Err("Short read".into());
        }
        Ok(BigEndian::read_u32(slice))
    }

    pub fn write_u32(&mut self, addr: u64, val: u32) -> Result<(), String> {
        if let Some(off) = mmio_offset(addr) {
            self.gpu_mmio.insert(off, val);
            self.gpu_mmio_writes += 1;
            return Ok(());
        }
        self.check_watch(addr, 4, val as u64);
        let slice = self.translate_address_mut(addr)?;
        if slice.len() < 4 {
            return Err("Short write".into());
        }
        BigEndian::write_u32(slice, val);
        Ok(())
    }

    pub fn read_u32_le(&self, addr: u64) -> Result<u32, String> {
        let slice = self.translate_address(addr)?;
        if slice.len() < 4 {
            return Err("Short read".into());
        }
        Ok(byteorder::LittleEndian::read_u32(slice))
    }

    pub fn read_u64(&self, addr: u64) -> Result<u64, String> {
        let slice = self.translate_address(addr)?;
        if slice.len() < 8 {
            return Err("Short read".into());
        }
        Ok(BigEndian::read_u64(slice))
    }

    pub fn write_u64(&mut self, addr: u64, val: u64) -> Result<(), String> {
        self.check_watch(addr, 8, val);
        let slice = self.translate_address_mut(addr)?;
        if slice.len() < 8 {
            return Err("Short write".into());
        }
        BigEndian::write_u64(slice, val);
        Ok(())
    }

    /// Load raw bytes directly into memory at a given physical address
    pub fn load_bytes(&mut self, start_addr: u64, data: &[u8]) -> Result<(), String> {
        for (i, &b) in data.iter().enumerate() {
            self.write_u8(start_addr + i as u64, b)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_mmio_fixed_reads_and_write_back() {
        let mut bus = MemoryBus::new();
        // 0x7FC86544 = register 0x1951 (read by the dash's interrupt callback at 0x921AFD18)
        assert_eq!(bus.read_u32(0x7FC8_6544).unwrap(), 1);
        bus.write_u32(0x7FC8_6544, 0).unwrap();
        assert_eq!(bus.read_u32(0x7FC8_6544).unwrap(), 1, "fixed registers ignore writes on read");
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x0F00 * 4).unwrap(), 0x0810_0748);
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x1961 * 4).unwrap(), 0x0500_02D0);
        // other registers read back what was written (CP_RB_WPTR)
        bus.write_u32(0x7FC8_0000 + GPU_MMIO_CP_RB_WPTR as u64, 49).unwrap();
        assert_eq!(bus.read_u32(0x7FC8_0714).unwrap(), 49);
        assert_eq!(bus.gpu_mmio.get(&GPU_MMIO_CP_RB_WPTR), Some(&49));
    }
}
