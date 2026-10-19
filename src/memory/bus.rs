use byteorder::{BigEndian, ByteOrder};

/// Xbox 360 Physical Memory Bus
pub struct MemoryBus {
    /// 512 MB Main GDDR3 / DDR3 RAM
    pub ram: Vec<u8>,
    /// 512 KB Internal CPU SRAM / L2 Cache (used as Cache-As-RAM during 1BL and CB stages)
    pub sram: Vec<u8>,
}

const RAM_SIZE: usize = 512 * 1024 * 1024; // 512 MB
const SRAM_SIZE: usize = 512 * 1024;       // 512 KB
const SRAM_PREFIX: u64 = 0x8000_0200_0000_0000;

impl MemoryBus {
    pub fn new() -> Self {
        Self {
            ram: vec![0u8; RAM_SIZE],
            sram: vec![0u8; SRAM_SIZE],
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

        let ram_offset = (addr & 0x1FFF_FFFF) as usize;
        if ram_offset < RAM_SIZE {
            Ok(&self.ram[ram_offset..])
        } else {
            Err(format!("Physical RAM access out of bounds: 0x{:016X}", addr))
        }
    }

    pub fn read_u8(&self, addr: u64) -> Result<u8, String> {
        let slice = self.translate_address(addr)?;
        Ok(slice[0])
    }

    pub fn write_u8(&mut self, addr: u64, val: u8) -> Result<(), String> {
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
        let slice = self.translate_address_mut(addr)?;
        if slice.len() < 2 {
            return Err("Short write".into());
        }
        BigEndian::write_u16(slice, val);
        Ok(())
    }

    pub fn read_u32(&self, addr: u64) -> Result<u32, String> {
        let slice = self.translate_address(addr)?;
        if slice.len() < 4 {
            return Err("Short read".into());
        }
        Ok(BigEndian::read_u32(slice))
    }

    pub fn write_u32(&mut self, addr: u64, val: u32) -> Result<(), String> {
        let slice = self.translate_address_mut(addr)?;
        if slice.len() < 4 {
            return Err("Short write".into());
        }
        BigEndian::write_u32(slice, val);
        Ok(())
    }

    pub fn read_u64(&self, addr: u64) -> Result<u64, String> {
        let slice = self.translate_address(addr)?;
        if slice.len() < 8 {
            return Err("Short read".into());
        }
        Ok(BigEndian::read_u64(slice))
    }

    pub fn write_u64(&mut self, addr: u64, val: u64) -> Result<(), String> {
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
