/// PowerPC 64-bit Xenon CPU Register State
#[derive(Debug, Clone)]
pub struct Registers {
    /// 32 General Purpose Registers (64-bit each)
    pub gpr: [u64; 32],
    /// Program Counter (Next Instruction Pointer)
    pub pc: u64,
    /// Link Register
    pub lr: u64,
    /// Count Register (used for loops and branch target)
    pub ctr: u64,
    /// Condition Register (32 bits, 8 fields of 4 bits: CR0..CR7)
    pub cr: u32,
    /// Fixed-Point Exception Register
    pub xer: u64,
    /// Machine State Register
    pub msr: u64,
    /// 32 Floating-Point Registers (64-bit IEEE 754 each)
    pub fpr: [f64; 32],
    /// Floating-Point Status and Control Register
    pub fpscr: u64,
    /// 128 Vector Registers (128-bit / 16 bytes each on Xenon VMX128)
    pub vr: [[u8; 16]; 128],
    /// Vector Status and Control Register
    pub vscr: u32,
    /// Vector Register Save Register
    pub vrsave: u32,
}

impl Registers {
    pub fn new() -> Self {
        Self {
            gpr: [0u64; 32],
            pc: 0,
            lr: 0,
            ctr: 0,
            cr: 0,
            xer: 0,
            msr: 0,
            fpr: [0.0; 32],
            fpscr: 0,
            vr: [[0u8; 16]; 128],
            vscr: 0,
            vrsave: 0,
        }
    }

    /// Update Condition Register Field (crf 0..7) comparing two 32-bit unsigned integers
    pub fn update_cr_cmplw(&mut self, crf: u32, a: u32, b: u32) {
        let shift = (7 - crf) * 4;
        let mask = !(0xFu32 << shift);
        let bits = if a < b {
            0b1000 // Negative / Less than
        } else if a > b {
            0b0100 // Positive / Greater than
        } else {
            0b0010 // Equal
        };
        // Also bit 0b0001 is SO (summary overflow from XER)
        self.cr = (self.cr & mask) | (bits << shift);
    }

    /// Update Condition Register Field (crf 0..7) comparing two 32-bit signed integers
    pub fn update_cr_cmpw(&mut self, crf: u32, a: i32, b: i32) {
        let shift = (7 - crf) * 4;
        let mask = !(0xFu32 << shift);
        let bits = if a < b {
            0b1000
        } else if a > b {
            0b0100
        } else {
            0b0010
        };
        self.cr = (self.cr & mask) | (bits << shift);
    }

    /// Update Condition Register Field (crf 0..7) comparing two 64-bit signed integers
    pub fn update_cr_cmpd(&mut self, crf: u32, a: i64, b: i64) {
        let shift = (7 - crf) * 4;
        let mask = !(0xFu32 << shift);
        let bits = if a < b {
            0b1000
        } else if a > b {
            0b0100
        } else {
            0b0010
        };
        self.cr = (self.cr & mask) | (bits << shift);
    }

    /// Update Condition Register Field (crf 0..7) comparing two 64-bit floating point numbers
    pub fn update_cr_cmpf(&mut self, crf: u32, a: f64, b: f64) {
        let shift = (7 - crf) * 4;
        let mask = !(0xFu32 << shift);
        let bits = if a.is_nan() || b.is_nan() {
            0b0001
        } else if a < b {
            0b1000
        } else if a > b {
            0b0100
        } else {
            0b0010
        };
        self.cr = (self.cr & mask) | (bits << shift);
    }

    pub fn get_cr_bit(&self, bit: u32) -> bool {
        ((self.cr >> (31 - bit)) & 1) != 0
    }

    pub fn dump_key_registers(&self) {
        println!(
            "PC: 0x{:016X} | LR: 0x{:016X} | CTR: 0x{:016X} | CR: 0x{:08X}",
            self.pc, self.lr, self.ctr, self.cr
        );
        println!(
            "r1 (SP): 0x{:016X} | r2 (TOC/Base): 0x{:016X} | r3: 0x{:016X}",
            self.gpr[1], self.gpr[2], self.gpr[3]
        );
        println!(
            "r4: 0x{:016X} | r5: 0x{:016X} | r26: 0x{:016X} | r31: 0x{:016X}",
            self.gpr[4], self.gpr[5], self.gpr[26], self.gpr[31]
        );
    }
}
