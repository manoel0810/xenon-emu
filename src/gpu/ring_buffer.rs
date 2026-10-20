use crate::memory::MemoryBus;

/// Emulates the ATI Xenos (Xbox 360) Primary Ring Buffer in guest memory
pub struct RingBuffer {
    pub base_addr: u64,
    pub num_dwords: usize,
    pub wptr: usize,
    pub rptr: usize,
}

impl RingBuffer {
    /// Creates a new primary ring buffer at base address with size in bytes
    pub fn new(base_addr: u64, size_bytes: usize) -> Self {
        let num_dwords = size_bytes / 4;
        Self {
            base_addr,
            num_dwords,
            wptr: 0,
            rptr: 0,
        }
    }

    /// Returns true if there are unconsumed commands in the ring buffer
    pub fn has_pending(&self) -> bool {
        self.rptr != self.wptr
    }

    /// Returns the number of unconsumed 32-bit DWORDs currently in the buffer
    pub fn pending_dwords(&self) -> usize {
        if self.wptr >= self.rptr {
            self.wptr - self.rptr
        } else {
            self.num_dwords - (self.rptr - self.wptr)
        }
    }

    /// Writes a slice of 32-bit PM4 DWORDs into the ring buffer and advances WPTR
    pub fn write_packet(&mut self, bus: &mut MemoryBus, packet_words: &[u32]) -> Result<(), String> {
        for &w in packet_words {
            let addr = self.base_addr + (self.wptr * 4) as u64;
            bus.write_u32(addr, w)?;
            self.wptr = (self.wptr + 1) % self.num_dwords;
        }
        Ok(())
    }

    /// Reads up to `count` DWORDs starting at RPTR without advancing RPTR
    pub fn peek_words(&self, bus: &MemoryBus, count: usize) -> Result<Vec<u32>, String> {
        let mut words = Vec::with_capacity(count);
        let mut ptr = self.rptr;
        for _ in 0..count {
            let addr = self.base_addr + (ptr * 4) as u64;
            let val = bus.read_u32(addr)?;
            words.push(val);
            ptr = (ptr + 1) % self.num_dwords;
        }
        Ok(words)
    }

    /// Advances the Read Pointer (RPTR) by `count` DWORDs
    pub fn advance_rptr(&mut self, count: usize) {
        self.rptr = (self.rptr + count) % self.num_dwords;
    }

    /// Dynamically scans memory from wptr forward to discover newly written PM4 words from guest CPU
    pub fn sync_wptr_from_memory(&mut self, bus: &MemoryBus) {
        while self.wptr < self.num_dwords {
            let addr = self.base_addr + (self.wptr * 4) as u64;
            if bus.read_u32(addr).unwrap_or(0) != 0 {
                self.wptr += 1;
            } else {
                break;
            }
        }
    }
}
