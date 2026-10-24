// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/gpu/register_file.h        RegisterFile::kRegisterCount, values[]
//   src/xenia/gpu/graphics_system.cc     GraphicsSystem::ReadRegister / WriteRegister (MMIO 0x7FC80000)
//   src/xenia/gpu/command_processor.cc   CommandProcessor::WriteRegister (PM4 register writes)
//
//! The Xenos register file. One array of 32-bit registers, written both by the command processor (PM4 Type-0,
//! REG_RMW, ...) and by the CPU through the MMIO window at 0x7FC80000 (register = byte offset / 4), and read by both.

/// Number of registers (register_file.h).
pub const REGISTER_COUNT: usize = 0x5003;

/// CP_RB_WPTR: ring buffer write pointer, in dwords (MMIO byte offset 0x714).
pub const CP_RB_WPTR: u32 = 0x01C5;

pub struct RegisterFile {
    values: Box<[u32]>,
}

impl Default for RegisterFile {
    fn default() -> Self {
        Self { values: vec![0u32; REGISTER_COUNT].into_boxed_slice() }
    }
}

impl RegisterFile {
    /// Current value; registers past REGISTER_COUNT read 0.
    pub fn get(&self, reg: u32) -> u32 {
        self.values.get(reg as usize).copied().unwrap_or(0)
    }

    /// Stores a value; writes past REGISTER_COUNT are dropped (Xenia logs and ignores them).
    pub fn set(&mut self, reg: u32, value: u32) {
        if let Some(v) = self.values.get_mut(reg as usize) {
            *v = value;
        }
    }

    /// A CPU read through the MMIO window (GraphicsSystem::ReadRegister): a few registers return fixed values,
    /// the rest read the register file.
    pub fn mmio_read(&self, reg: u32) -> u32 {
        match reg {
            0x0F00 => 0x0810_0748, // RB_EDRAM_TIMING
            0x0F01 => 0x0000_200E, // RB_BC_CONTROL
            0x0F2D => 0x00BB_BBBB, // RB_HSIO_INTERFACE_ALIGNER_VALUE
            0x1951 => 1,           // interrupt status: vblank (the dash's interrupt callback checks bit 0)
            0x1961 => 0x0500_02D0, // AVIVO_D1MODE_VIEWPORT_SIZE: 1280x720
            _ => self.get(reg),
        }
    }

    /// Registers holding a non-zero value, in register order (diagnostics).
    pub fn non_zero(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.values.iter().enumerate().filter(|(_, v)| **v != 0).map(|(r, &v)| (r as u32, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_mmio_reads_and_bounds() {
        let mut rf = RegisterFile::default();
        rf.set(0x1951, 0);
        assert_eq!(rf.mmio_read(0x1951), 1, "fixed registers ignore the stored value on MMIO reads");
        assert_eq!(rf.get(0x1951), 0, "but the CP sees what was written");
        assert_eq!((rf.mmio_read(0x0F00), rf.mmio_read(0x1961)), (0x0810_0748, 0x0500_02D0));
        rf.set(REGISTER_COUNT as u32, 7);
        assert_eq!(rf.get(REGISTER_COUNT as u32), 0);
        rf.set(0x2000, 5);
        assert_eq!(rf.non_zero().collect::<Vec<_>>(), vec![(0x2000, 5)]);
    }
}
