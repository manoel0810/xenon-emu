//! xboxkrnl.exe HLE: XeCrypt*/XeKeys*: crypto and key vault.

use super::super::dispatcher::KernelDispatcher;
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_xe_crypt(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        Some(match ordinal {
            // 0x0166: XeCryptBnQwBeSigVerify
            0x0166 => 1, // Signature Valid (TRUE)

            // XeCryptDes* exports 0x0174..0x017A
            0x0174 | 0x0175 | 0x0176 | 0x0178 | 0x017A => 0,

            // 0x0182: XeCryptHmacSha | 0x0183: XeCryptHmacShaVerify
            0x0182 | 0x0183 => 1,

            // 0x0187: XeCryptMd5 | 0x018E: XeCryptRotSumSha | 0x0192: XeCryptSha
            0x0187 | 0x018E | 0x0192 => 0,

            // 0x018A: XeCryptRandom (pBuffer, cbBuffer) per the generated export table; the dash calls it 4096x with (buf, 4).
            // (An older comment here called it VeSetHandlers.) Fill the buffer with deterministic xorshift bytes.
            0x018A => {
                let p = args.first().copied().unwrap_or(0);
                let n = (args.get(1).copied().unwrap_or(0) & 0xFFFF_FFFF).min(4096);
                if p != 0 {
                    let mut x = self.rng_state;
                    for i in 0..n {
                        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
                        let _ = bus.write_u8(p + i, (x >> 24) as u8);
                    }
                    self.rng_state = x;
                }
                0
            }

            // 0x0244: XeKeysGetKey (keyIndex, pBuffer)
            0x0244 => {
                if let Some(&p_buf) = args.get(1) {
                    if p_buf != 0 {
                        for off in (0..16).step_by(4) {
                            let _ = bus.write_u32(p_buf + off, 0);
                        }
                    }
                }
                0
            }

            // 0x0246: XeKeysGetConsoleID (pConsoleId)
            0x0246 => {
                if let Some(&p_id) = args.first() {
                    if p_id != 0 {
                        let _ = bus.write_u32(p_id, 0x0020_11B4);
                        let _ = bus.write_u32(p_id + 4, 0x0000_0001);
                    }
                }
                0
            }

            _ => return None,
        })
    }
}
