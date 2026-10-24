//! Minimal guest-memory interface for HLE code that should be testable without a full `MemoryBus`.

use crate::memory::MemoryBus;

/// Guest memory access (big-endian, like the console). Unmapped reads return 0; failed writes are dropped.
pub trait GuestMem {
    fn read_u8(&self, addr: u64) -> u8;
    fn read_u16(&self, addr: u64) -> u16;
    fn read_u32(&self, addr: u64) -> u32;
    fn write_u8(&mut self, addr: u64, v: u8);
    fn write_u16(&mut self, addr: u64, v: u16);
    fn write_u32(&mut self, addr: u64, v: u32);

    fn read_u64(&self, addr: u64) -> u64 {
        ((self.read_u32(addr) as u64) << 32) | self.read_u32(addr + 4) as u64
    }

    /// NUL-terminated narrow string (at most `max` bytes).
    fn read_cstr(&self, addr: u64, max: usize) -> Vec<u8> {
        (0..max as u64).map(|i| self.read_u8(addr + i)).take_while(|&b| b != 0).collect()
    }
}

impl GuestMem for MemoryBus {
    fn read_u8(&self, addr: u64) -> u8 { MemoryBus::read_u8(self, addr).unwrap_or(0) }
    fn read_u16(&self, addr: u64) -> u16 { MemoryBus::read_u16(self, addr).unwrap_or(0) }
    fn read_u32(&self, addr: u64) -> u32 { MemoryBus::read_u32(self, addr).unwrap_or(0) }
    fn write_u8(&mut self, addr: u64, v: u8) { let _ = MemoryBus::write_u8(self, addr, v); }
    fn write_u16(&mut self, addr: u64, v: u16) { let _ = MemoryBus::write_u16(self, addr, v); }
    fn write_u32(&mut self, addr: u64, v: u32) { let _ = MemoryBus::write_u32(self, addr, v); }
}

/// Sparse in-memory guest memory for unit tests.
#[cfg(test)]
pub mod fake {
    use super::GuestMem;
    use std::collections::HashMap;

    #[derive(Default)]
    pub struct FakeMem(HashMap<u64, u8>);

    impl GuestMem for FakeMem {
        fn read_u8(&self, a: u64) -> u8 { *self.0.get(&a).unwrap_or(&0) }
        fn read_u16(&self, a: u64) -> u16 { ((self.read_u8(a) as u16) << 8) | self.read_u8(a + 1) as u16 }
        fn read_u32(&self, a: u64) -> u32 { ((self.read_u16(a) as u32) << 16) | self.read_u16(a + 2) as u32 }
        fn write_u8(&mut self, a: u64, v: u8) { self.0.insert(a, v); }
        fn write_u16(&mut self, a: u64, v: u16) { self.write_u8(a, (v >> 8) as u8); self.write_u8(a + 1, v as u8); }
        fn write_u32(&mut self, a: u64, v: u32) { self.write_u16(a, (v >> 16) as u16); self.write_u16(a + 2, v as u16); }
    }

    impl FakeMem {
        pub fn put_str(&mut self, a: u64, s: &str) { for (i, b) in s.bytes().chain([0]).enumerate() { self.write_u8(a + i as u64, b); } }
        pub fn put_wstr(&mut self, a: u64, s: &str) { for (i, c) in s.encode_utf16().chain([0]).enumerate() { self.write_u16(a + 2 * i as u64, c); } }
        pub fn get_str(&self, a: u64) -> String { (0..).map(|i| self.read_u8(a + i)).take_while(|&b| b != 0).map(|b| b as char).collect() }
        pub fn get_wstr(&self, a: u64) -> String {
            let v: Vec<u16> = (0..).map(|i| self.read_u16(a + 2 * i)).take_while(|&c| c != 0).collect();
            String::from_utf16_lossy(&v)
        }
    }
}
