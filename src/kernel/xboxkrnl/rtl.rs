//! xboxkrnl.exe HLE: Rtl*: run-time library (strings, memory, critical sections, time).

use super::super::dispatcher::KernelDispatcher;
use super::super::strings::{self, ArrayArgs, StackArgs};
use crate::memory::MemoryBus;

impl KernelDispatcher {
    /// Handles the ordinals of this area; `None` means "not handled here".
    pub(super) fn xboxkrnl_rtl(&mut self, ordinal: u16, args: &[u64], bus: &mut MemoryBus) -> Option<u64> {
        let arg = |i: usize| args.get(i).copied().unwrap_or(0);
        // r1 of the calling thread, for variadic arguments beyond r10
        let sp = bus.current_sp;
        Some(match ordinal {
            // 0x0119: RtlCaptureContext
            0x0119 => 0,

            // 0x011B: RtlCompareMemoryUlong
            0x011B => args.get(1).copied().unwrap_or(0),

            // 0x0125: RtlEnterCriticalSection
            0x0125 => 0,

            // 0x0126: RtlFillMemoryUlong (Destination, Length in BYTES, Pattern)
            // (call site in the dash: r3=dst, r4=800, r5=0x80000000 -> Length=800, Pattern=0x80000000.
            //  The previous handler read (dst, pattern, count) and looped 2^64 times on the sign-extended pattern.)
            0x0126 => {
                let dst = args.first().copied().unwrap_or(0);
                let len = (args.get(1).copied().unwrap_or(0) & 0xFFFF_FFFF) as u64;
                let pat = args.get(2).copied().unwrap_or(0) as u32;
                // Defensive cap: a single fill can never legitimately exceed the whole 512 MB RAM
                let words = (len / 4).min(128 * 1024 * 1024);
                for i in 0..words {
                    let _ = bus.write_u32(dst.wrapping_add(i * 4), pat);
                }
                0
            }
            // 0x012B: RtlImageXexHeaderField
            0x012B => 0,

            // 0x012C: RtlInitAnsiString (DestinationString, SourceString)
            0x012C => {
                let dst = args.first().copied().unwrap_or(0);
                let src = args.get(1).copied().unwrap_or(0);
                if dst != 0 {
                    if src == 0 {
                        let _ = bus.write_u16(dst, 0);
                        let _ = bus.write_u16(dst + 2, 0);
                        let _ = bus.write_u32(dst + 4, 0);
                    } else {
                        let mut len = 0u16;
                        while len < 0xFFFE {
                            if bus.read_u8(src + len as u64).unwrap_or(0) == 0 {
                                break;
                            }
                            len += 1;
                        }
                        let _ = bus.write_u16(dst, len);
                        let _ = bus.write_u16(dst + 2, len.saturating_add(1));
                        let _ = bus.write_u32(dst + 4, src as u32);
                    }
                }
                0
            }

            // 0x012D: RtlInitUnicodeString (DestinationString, SourceString)
            0x012D => {
                let dst = args.first().copied().unwrap_or(0);
                let src = args.get(1).copied().unwrap_or(0);
                if dst != 0 {
                    if src == 0 {
                        let _ = bus.write_u16(dst, 0);
                        let _ = bus.write_u16(dst + 2, 0);
                        let _ = bus.write_u32(dst + 4, 0);
                    } else {
                        let mut len_bytes = 0u16;
                        while len_bytes < 0xFFFC {
                            let ch = bus.read_u16(src + len_bytes as u64).unwrap_or(0);
                            if ch == 0 {
                                break;
                            }
                            len_bytes += 2;
                        }
                        let _ = bus.write_u16(dst, len_bytes);
                        let _ = bus.write_u16(dst + 2, len_bytes.saturating_add(2));
                        let _ = bus.write_u32(dst + 4, src as u32);
                    }
                }
                0
            }

            // 0x012E: RtlInitializeCriticalSection | 0x012F: RtlInitializeCriticalSectionAndSpinCount
            0x012E | 0x012F => {
                if let Some(&ptr) = args.first() {
                    if ptr != 0 {
                        for off in (0..28).step_by(4) {
                            let _ = bus.write_u32(ptr + off, 0);
                        }
                    }
                }
                0
            }

            // 0x0130: RtlLeaveCriticalSection
            0x0130 => 0,

            // 0x0133: RtlMultiByteToUnicodeN
            0x0133 => 0,

            // 0x0135: RtlNtStatusToDosError
            0x0135 => 0, // ERROR_SUCCESS

            // 0x0136: RtlRaiseException
            0x0136 => 0,

            // printf family: see kernel/strings.rs (port of Xenia's xboxkrnl_strings.cc)
            // 0x013A: _snprintf(buffer, count, format, ...)
            0x013A => strings::snprintf(bus, arg(0), arg(1) as u32, arg(2), &mut StackArgs::new(args, sp, 3), false, false),
            // 0x013B: sprintf(buffer, format, ...)
            0x013B => strings::sprintf(bus, arg(0), arg(1), &mut StackArgs::new(args, sp, 2), false),
            // 0x013D: _snwprintf(buffer, count, format, ...)
            0x013D => strings::snprintf(bus, arg(0), arg(1) as u32, arg(2), &mut StackArgs::new(args, sp, 3), true, false),
            // 0x013E: swprintf(buffer, format, ...)
            0x013E => strings::sprintf(bus, arg(0), arg(1), &mut StackArgs::new(args, sp, 2), true),
            // 0x014D: _vsnprintf(buffer, count, format, va_list)
            0x014D => strings::snprintf(bus, arg(0), arg(1) as u32, arg(2), &mut ArrayArgs::new(arg(3)), false, false),
            // 0x014E: vsprintf(buffer, format, va_list)
            0x014E => strings::sprintf(bus, arg(0), arg(1), &mut ArrayArgs::new(arg(2)), false),
            // 0x014F: _vscwprintf(format, va_list)
            0x014F => strings::vscwprintf(bus, arg(0), &mut ArrayArgs::new(arg(1))),
            // 0x0150: _vsnwprintf(buffer, count, format, va_list): returns the full count on overflow
            0x0150 => strings::snprintf(bus, arg(0), arg(1) as u32, arg(2), &mut ArrayArgs::new(arg(3)), true, true),
            // 0x0151: vswprintf(buffer, format, va_list)
            0x0151 => strings::sprintf(bus, arg(0), arg(1), &mut ArrayArgs::new(arg(2)), true),

            // 0x0140: RtlFreeAnsiString | 0x0141: RtlFreeUnicodeString
            0x0140 | 0x0141 => {
                let str_ptr = args.first().copied().unwrap_or(0);
                if str_ptr != 0 {
                    let _ = bus.write_u16(str_ptr, 0);
                    let _ = bus.write_u16(str_ptr + 2, 0);
                    let _ = bus.write_u32(str_ptr + 4, 0);
                }
                0
            }

            // 0x0142: RtlUnicodeStringToAnsiString (DestinationString, SourceString, AllocateDestinationString)
            0x0142 => {
                let dst = args.first().copied().unwrap_or(0);
                let src = args.get(1).copied().unwrap_or(0);
                let alloc_dest = args.get(2).copied().unwrap_or(0) != 0;

                if dst == 0 || src == 0 {
                    0xC000000D // STATUS_INVALID_PARAMETER
                } else {
                    let src_len = bus.read_u16(src).unwrap_or(0);
                    let src_buf = bus.read_u32(src + 4).unwrap_or(0) as u64;
                    let char_count = (src_len / 2) as usize;

                    let mut ansi_bytes = Vec::with_capacity(char_count);
                    for i in 0..char_count {
                        let wch = bus.read_u16(src_buf + (i * 2) as u64).unwrap_or(0);
                        ansi_bytes.push(if wch < 0x80 { wch as u8 } else { b'?' });
                    }

                    let dst_buf = if alloc_dest {
                        let alloc_size = ((char_count + 1 + 31) & !31) as u64;
                        let ptr = self.valloc_bump;
                        self.valloc_bump += alloc_size;
                        let _ = bus.write_u16(dst + 2, (char_count + 1) as u16);
                        let _ = bus.write_u32(dst + 4, ptr as u32);
                        ptr
                    } else {
                        bus.read_u32(dst + 4).unwrap_or(0) as u64
                    };

                    let _ = bus.write_u16(dst, char_count as u16);
                    if dst_buf != 0 {
                        for (i, &b) in ansi_bytes.iter().enumerate() {
                            let _ = bus.write_u8(dst_buf + i as u64, b);
                        }
                        let _ = bus.write_u8(dst_buf + char_count as u64, 0);
                    }

                    0 // STATUS_SUCCESS
                }
            }

            // 0x0143: RtlUnicodeToMultiByteN
            0x0143 => 0,

            // 0x0147: RtlUnwind
            0x0147 => 0,


            _ => return None,
        })
    }
}
