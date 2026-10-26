//! xboxkrnl.exe HLE: Rtl*: run-time library (strings, memory, critical sections, time).

use super::super::dispatcher::KernelDispatcher;
use super::super::strings::{self, ArrayArgs, StackArgs};
use crate::kernel::{objects, scheduler};
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

            // 0x0125: RtlEnterCriticalSection(cs): taken at once when free or already owned; otherwise the thread waits on
            // the section's event and takes it when woken (Xenia xboxkrnl_rtl.cc; the spin loop has nothing to spin on
            // here, where one guest thread runs at a time)
            0x0125 => {
                let cs = arg(0);
                if cs != 0 && !objects::enter_critical_section(bus, cs, self.current_kthread) {
                    let mut wait = scheduler::WaitRequest::objects(self.current_kthread, vec![cs as u32], false, None);
                    wait.on_wake = scheduler::WakeAction::EnterCriticalSection(cs as u32);
                    self.begin_wait(bus, wait);
                }
                0
            }

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

            // 0x012E: RtlInitializeCriticalSection(cs) | 0x012F: RtlInitializeCriticalSectionAndSpinCount(cs, spin_count)
            // (xeRtlInitializeCriticalSection*: spin count / 256, rounded up, kept in header.absolute)
            0x012E | 0x012F => {
                let cs = arg(0);
                if cs != 0 {
                    let spin = if ordinal == 0x012F { ((arg(1) + 255) >> 8).min(255) as u8 } else { 0 };
                    objects::init_critical_section(bus, cs, spin);
                }
                0
            }

            // 0x0130: RtlLeaveCriticalSection(cs)
            0x0130 => {
                if arg(0) != 0 {
                    objects::leave_critical_section(bus, arg(0));
                }
                0
            }

            // 0x0133: RtlMultiByteToUnicodeN(dst, dst_len_bytes, written_ptr, src, src_len) - see kernel/strings.rs
            0x0133 => strings::rtl_multibyte_to_unicode_n(bus, arg(0), arg(1) as u32, arg(2), arg(3), arg(4) as u32),

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

            // 0x0127: RtlFreeAnsiString | 0x0128: RtlFreeUnicodeString (string): the string is reset (length,
            // maximum length, pointer = 0); the buffer itself is not returned to a heap here
            0x0127 | 0x0128 => {
                let str_ptr = args.first().copied().unwrap_or(0);
                if str_ptr != 0 {
                    let _ = bus.write_u16(str_ptr, 0);
                    let _ = bus.write_u16(str_ptr + 2, 0);
                    let _ = bus.write_u32(str_ptr + 4, 0);
                }
                0
            }

            // 0x0140: RtlTimeToTimeFields(time_ptr, time_fields_ptr) -> void (X_TIME_FIELDS, Xenia xboxkrnl_rtl.cc)
            0x0140 => {
                let (time_ptr, fields_ptr) = (arg(0), arg(1));
                if time_ptr != 0 && fields_ptr != 0 {
                    let fields = time_to_time_fields(bus.read_u64(time_ptr).unwrap_or(0));
                    for (i, v) in fields.iter().enumerate() {
                        let _ = bus.write_u16(fields_ptr + 2 * i as u64, *v);
                    }
                }
                0
            }

            // 0x0141: RtlTryEnterCriticalSection(cs) -> TRUE if the section is now held (free, or already owned)
            0x0141 => {
                let cs = arg(0);
                if cs == 0 {
                    return Some(1); // Xenia: pretend we got it
                }
                let thread = self.current_kthread;
                let lock_count = bus.read_u32(cs + objects::CS_LOCK_COUNT).unwrap_or(0) as i32;
                if lock_count == -1 {
                    let _ = bus.write_u32(cs + objects::CS_LOCK_COUNT, 0);
                    objects::take_critical_section(bus, cs, thread);
                    1
                } else if thread != 0 && bus.read_u32(cs + objects::CS_OWNING_THREAD).unwrap_or(0) == thread {
                    objects::enter_critical_section(bus, cs, thread);
                    1
                } else {
                    0
                }
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

            // 0x0143: RtlUnicodeToMultiByteN(dst, dst_len, written_ptr, src, src_len_bytes) - see kernel/strings.rs
            0x0143 => strings::rtl_unicode_to_multibyte_n(bus, arg(0), arg(1) as u32, arg(2), arg(3), arg(4) as u32),

            // 0x0147: RtlUnwind
            0x0147 => 0,


            _ => return None,
        })
    }
}

/// RtlTimeToTimeFields: a FILETIME (100 ns units since 1601-01-01) as X_TIME_FIELDS
/// [year, month, day, hour, minute, second, milliseconds, weekday (0 = Sunday)].
fn time_to_time_fields(filetime: u64) -> [u16; 8] {
    let ms_total = filetime / 10_000;
    let days = (ms_total / 86_400_000) as i64;
    let ms_of_day = ms_total % 86_400_000;
    // civil date from days since 1601-01-01 (shifted to days since 1970-01-01; Howard Hinnant's algorithm)
    let z = days - 134_774 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    // 1601-01-01 was a Monday
    let weekday = (days + 1).rem_euclid(7);
    [
        year as u16,
        month as u16,
        day as u16,
        (ms_of_day / 3_600_000) as u16,
        (ms_of_day / 60_000 % 60) as u16,
        (ms_of_day / 1000 % 60) as u16,
        (ms_of_day % 1000) as u16,
        weekday as u16,
    ]
}

#[cfg(test)]
mod tests {
    use super::time_to_time_fields;
    use crate::kernel::KernelDispatcher;
    use crate::memory::MemoryBus;

    #[test]
    fn time_fields_from_filetime() {
        // 1601-01-01 00:00:00.000 was a Monday
        assert_eq!(time_to_time_fields(0), [1601, 1, 1, 0, 0, 0, 0, 1]);
        // 2000-01-01 00:00:00 (Unix 946684800), a Saturday
        assert_eq!(time_to_time_fields(116_444_736_000_000_000 + 946_684_800 * 10_000_000), [2000, 1, 1, 0, 0, 0, 0, 6]);
        // 2024-02-29 13:45:30.250 (Unix 1709214330.25), a Thursday
        assert_eq!(time_to_time_fields(116_444_736_000_000_000 + 1_709_214_330_250 * 10_000), [2024, 2, 29, 13, 45, 30, 250, 4]);
    }

    #[test]
    fn rtl_ordinals_do_what_their_names_say() {
        let mut k = KernelDispatcher::new();
        let mut bus = MemoryBus::new();
        // RtlTryEnterCriticalSection on a section held by another thread fails and leaves it untouched (it used to be
        // "RtlFreeUnicodeString", which zeroed it)
        for off in (0..0x1C).step_by(4) {
            bus.write_u32(0x4000_0000 + off, 0xEEEE_EEEE).unwrap();
        }
        k.current_kthread = 0x4100_0000;
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0141, &[0x4000_0000], &mut bus), 0);
        assert_eq!(bus.read_u32(0x4000_0004).unwrap(), 0xEEEE_EEEE);
        // free (initialized) section: taken; again by the same thread: recursion
        k.dispatch("xboxkrnl.exe", 0x012E, &[0x4000_0040], &mut bus);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0141, &[0x4000_0040], &mut bus), 1);
        assert_eq!(k.dispatch("xboxkrnl.exe", 0x0141, &[0x4000_0040], &mut bus), 1);
        assert_eq!((bus.read_u32(0x4000_0050).unwrap(), bus.read_u32(0x4000_0054).unwrap(), bus.read_u32(0x4000_0058).unwrap()), (1, 2, 0x4100_0000));
        // RtlTimeToTimeFields
        bus.write_u64(0x4000_0100, 116_444_736_000_000_000 + 946_684_800 * 10_000_000).unwrap();
        k.dispatch("xboxkrnl.exe", 0x0140, &[0x4000_0100, 0x4000_0200], &mut bus);
        assert_eq!((bus.read_u16(0x4000_0200).unwrap(), bus.read_u16(0x4000_020E).unwrap()), (2000, 6));
        // RtlFreeAnsiString resets the string
        bus.write_u32(0x4000_0300, 0x0005_0010).unwrap();
        bus.write_u32(0x4000_0304, 0x4000_0400).unwrap();
        k.dispatch("xboxkrnl.exe", 0x0127, &[0x4000_0300], &mut bus);
        assert_eq!((bus.read_u32(0x4000_0300).unwrap(), bus.read_u32(0x4000_0304).unwrap()), (0, 0));
    }
}
