//! xboxkrnl.exe HLE: Mm*: memory manager (virtual/physical allocation, queries).

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_mm(&mut self, ordinal: u16, args: &[u64], _bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x00BA: MmAllocatePhysicalMemoryEx
            // (type, size, protect, minAddr, maxAddr, alignment)
            0x00BA => {
                let size = args.get(1).copied().unwrap_or(0x1000).max(0x1000);
                let aligned = (size + 0xFFF) & !0xFFF;
                let addr = self.phys_bump;
                self.phys_bump += aligned;
                if self.phys_bump > 0x9200_0000 {
                    println!("    -> MmAllocatePhysicalMemoryEx: physical bump exceeded 0x92000000 (would overlap the dash image)");
                }
                if aligned == 0x398000 {
                    if self.frontbuffer_ptr == 0 {
                        self.frontbuffer_ptr = addr;
                        println!("    -> MmAllocatePhysicalMemoryEx: Set Frontbuffer @ 0x{:08X}", addr);
                    } else if self.backbuffer_ptr == 0 {
                        self.backbuffer_ptr = addr;
                        println!("    -> MmAllocatePhysicalMemoryEx: Set Backbuffer @ 0x{:08X}", addr);
                    }
                }
                println!("    -> MmAllocatePhysicalMemoryEx: Allocated 0x{:X} bytes at 0x{:08X}", aligned, addr);
                addr
            }

            // 0x00BD: MmFreePhysicalMemory
            0x00BD => 0,

            // 0x00BE: MmGetPhysicalAddress
            0x00BE => args.first().copied().unwrap_or(0) & 0x1FFF_FFFF,

            // 0x00C4: MmQueryAddressProtect
            0x00C4 => 0x04, // PAGE_READWRITE

            // 0x00C5: MmQueryAllocationSize | 0x00C7: MmQueryAllocationSize
            0x00C5 | 0x00C7 => 0x1000,

            // 0x00C8: MmQueryStatistics
            0x00C8 => 0,

            _ => return None,
        })
    }
}
