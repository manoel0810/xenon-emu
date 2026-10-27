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
    /// The Xenos register file, shared by the MMIO window 0x7FC80000..0x7FC8FFFF (register = offset / 4) and the
    /// command processor.
    pub gpu_regs: crate::gpu::register_file::RegisterFile,
    /// Last CP_RB_WPTR stored by the CPU (GraphicsSystem::WriteRegister -> UpdateWritePointer); None until the
    /// guest writes it.
    pub cp_rb_wptr: Option<u32>,
    /// Count of guest stores into the GPU MMIO window (diagnostics)
    pub gpu_mmio_writes: u64,
    /// Number of times the write watchpoint has matched (diagnostics)
    pub watch_hits: u64,
    /// Sparse backing for the guest virtual-alloc window (0x70000000..0x8FFFFFFF).
    /// Key = 64 KB chunk index inside the window, value = physical RAM offset of the chunk.
    window_map: Vec<u32>,
    window_pool_next: usize,
    /// Images of XEX modules loaded after boot (`map_image`): (first guest address, end, physical RAM offset).
    image_maps: Vec<(u64, u64, usize)>,
    image_pool_next: usize,
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

/// Physical backing for module images mapped after boot: between the KPCR / loader-entry blocks (phys 0x13100000..
/// 0x131F2000, above the dash image) and the virtual-alloc pool. `addr & 0x1FFFFFFF` of a module's image base can fall
/// in that pool (dashnui.xex: 0x9E510000 -> 0x1E510000), so its pages are remapped here.
const IMAGE_POOL_START: usize = 0x1320_0000;
const IMAGE_POOL_END: usize = WINDOW_POOL_START;

/// GPU register window mapped by the kernel at 0x7FC80000 (CP_RB_WPTR = +0x714 ...). It must NOT be backed by plain RAM.
pub const GPU_MMIO_BASE: u64 = 0x7FC8_0000;
pub const GPU_MMIO_SIZE: u64 = 0x1_0000;

fn mmio_offset(addr: u64) -> Option<u32> {
    let a = addr & 0xFFFF_FFFF;
    if a >= GPU_MMIO_BASE && a < GPU_MMIO_BASE + GPU_MMIO_SIZE { Some((a - GPU_MMIO_BASE) as u32) } else { None }
}

/// Guest effective addresses are 32-bit: the upper 32 bits of a 64-bit EA are ignored, as in Xenia's
/// ComputeMemoryAddress (x64_seq_memory.cc, "Clear the top 32 bits, as they are likely garbage"). Code relies on it,
/// e.g. Lua's index2adr in the 17559 dash adds a zero-extended negative offset and gets 0x1_xxxxxxxx.
#[inline(always)]
fn window_addr(addr: u64) -> Option<u64> {
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
            gpu_regs: Default::default(),
            cp_rb_wptr: None,
            gpu_mmio_writes: 0,
            watch_hits: 0,
            window_map: vec![u32::MAX; ((WINDOW_END - WINDOW_START) / WINDOW_CHUNK) as usize],
            window_pool_next: WINDOW_POOL_START,
            image_maps: Vec::new(),
            image_pool_next: IMAGE_POOL_START,
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

        if let Some(off) = self.image_offset(addr) {
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

        if let Some(off) = self.image_offset(addr) {
            return Ok(&self.ram[off..]);
        }

        let ram_offset = (addr & 0x1FFF_FFFF) as usize;
        if ram_offset < RAM_SIZE {
            Ok(&self.ram[ram_offset..])
        } else {
            Err(format!("Physical RAM access out of bounds: 0x{:016X}", addr))
        }
    }

    /// RAM offset of `addr` inside an image mapped with `map_image`.
    #[inline(always)]
    fn image_offset(&self, addr: u64) -> Option<usize> {
        if self.image_maps.is_empty() {
            return None;
        }
        let a = addr & 0xFFFF_FFFF;
        self.image_maps.iter().find(|&&(start, end, _)| a >= start && a < end).map(|&(start, _, phys)| phys + (a - start) as usize)
    }

    /// Gives `size` bytes of guest addresses from `base` (a module image) zeroed physical pages of their own.
    pub fn map_image(&mut self, base: u64, size: u64) -> Result<(), String> {
        let base = base & 0xFFFF_FFFF;
        let size = (size + 0xFFF) & !0xFFF;
        if self.image_maps.iter().any(|&(start, end, _)| base < end && start < base + size) {
            return Err(format!("image 0x{:08X}+0x{:X} overlaps a mapped image", base, size));
        }
        let phys = self.image_pool_next;
        if phys + size as usize > IMAGE_POOL_END {
            return Err(format!("image pool exhausted mapping 0x{:08X}+0x{:X}", base, size));
        }
        self.ram[phys..phys + size as usize].fill(0);
        self.image_pool_next += size as usize;
        self.image_maps.push((base, base + size, phys));
        Ok(())
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
            return Ok(self.gpu_regs.mmio_read(off / 4));
        }
        let slice = self.translate_address(addr)?;
        if slice.len() < 4 {
            return Err("Short read".into());
        }
        Ok(BigEndian::read_u32(slice))
    }

    pub fn write_u32(&mut self, addr: u64, val: u32) -> Result<(), String> {
        if let Some(off) = mmio_offset(addr) {
            let reg = off / 4;
            if crate::diag::config().mmio_write_log {
                println!("  [MMIO W] reg=0x{:04X} val=0x{:08X} pc=0x{:08X} lr=0x{:08X} thread='{}'", reg, val,
                    self.current_pc & 0xFFFF_FFFF, self.current_lr & 0xFFFF_FFFF, self.current_thread);
            }
            self.gpu_regs.set(reg, val);
            if reg == crate::gpu::register_file::CP_RB_WPTR {
                self.cp_rb_wptr = Some(val);
            }
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

    /// Read raw bytes from memory at a given address into a buffer
    pub fn read_bytes(&self, start_addr: u64, buf: &mut [u8]) -> Result<(), String> {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.read_u8(start_addr + i as u64)?;
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
        // other registers read back what was written (CP_RB_WPTR), and the CP sees the same register file
        assert_eq!(bus.cp_rb_wptr, None);
        bus.write_u32(0x7FC8_0714, 49).unwrap();
        assert_eq!(bus.read_u32(0x7FC8_0714).unwrap(), 49);
        assert_eq!((bus.gpu_regs.get(0x1C5), bus.cp_rb_wptr), (49, Some(49)));
        bus.gpu_regs.set(0x2000, 0xABCD);
        assert_eq!(bus.read_u32(0x7FC8_0000 + 0x2000 * 4).unwrap(), 0xABCD, "CP register writes are visible over MMIO");
    }

    #[test]
    fn effective_addresses_ignore_the_upper_32_bits() {
        let mut bus = MemoryBus::new();
        // virtual-alloc window: 0x42DFB660 reached as top + zero-extended (-4 << 4) = 0x1_42DFB660
        bus.write_u32(0x42DF_B668, 5).unwrap();
        assert_eq!(bus.read_u32(0x1_42DF_B668).unwrap(), 5);
        bus.write_u32(0x1_42DF_B66C, 7).unwrap();
        assert_eq!(bus.read_u32(0xFFFF_FFFF_42DF_B66C).unwrap(), 7);
        // image / physical range
        bus.write_u32(0x92A7_809C, 0x1234).unwrap();
        assert_eq!(bus.read_u32(0x1_92A7_809C).unwrap(), 0x1234);
    }

    #[test]
    fn mapped_images_do_not_alias_the_virtual_alloc_pool() {
        let mut bus = MemoryBus::new();
        // dashnui.xex at 0x9E510000 masks to phys 0x1E510000, inside the window pool
        let window = 0x4000_0000 + (0x1E51_0000 - WINDOW_POOL_START as u64);
        for chunk in (0x4000_0000..=window).step_by(WINDOW_CHUNK as usize) {
            bus.write_u32(chunk, 1).unwrap();
        }
        bus.write_u32(window, 0xAAAA).unwrap();
        bus.map_image(0x9E51_0000, 0x34_4000).unwrap();
        assert_eq!(bus.read_u32(0x9E51_0000).unwrap(), 0, "fresh zeroed pages");
        bus.write_u32(0x9E51_0000, 0x5555).unwrap();
        bus.write_u32(0x9E85_3FFC, 0x6666).unwrap();
        assert_eq!((bus.read_u32(window).unwrap(), bus.read_u32(0x1_9E51_0000).unwrap()), (0xAAAA, 0x5555));
        assert_eq!(bus.read_u32(0x9E85_3FFC).unwrap(), 0x6666);
        assert!(bus.map_image(0x9E60_0000, 0x1000).is_err(), "overlapping image");
        assert!(bus.map_image(0x9F00_0000, 0x100_0000).is_err(), "pool exhausted");
    }
}
