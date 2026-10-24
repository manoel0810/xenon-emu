// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md.
// Port of src/xenia/kernel/xboxkrnl/xboxkrnl_strings.{h,cc} and util::get_arg_64 (shim_utils.h),
// Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de.
//
//! xboxkrnl printf family (_snprintf, sprintf, _snwprintf, swprintf, _vsnprintf, vsprintf, _vsnwprintf,
//! vswprintf, _vscwprintf). Formatting follows the MSVC CRT rules ("Format Specification Syntax: printf and
//! wprintf Functions"), as Xenia assumes for the 360 kernel. In the wide functions `%s` reads a wide string and
//! `%S` a narrow one; `h`/`l`/`w` force the width explicitly.

pub use super::guest_mem::GuestMem;

// ---- argument sources ----

/// Variadic arguments: index 0..7 come from r3..r10, the rest from the caller's stack (util::get_arg_64).
pub struct StackArgs<'a> {
    regs: &'a [u64],
    sp: u64,
    index: usize,
}

impl<'a> StackArgs<'a> {
    /// `regs` holds r3..r10 (missing entries read as 0); `first` is the index of the first variadic argument.
    pub fn new(regs: &'a [u64], sp: u64, first: usize) -> Self {
        Self { regs, sp, index: first }
    }
}

/// `va_list` arguments: consecutive 64-bit big-endian slots at `arg_ptr` (ArrayArgList).
pub struct ArrayArgs {
    arg_ptr: u64,
    index: u64,
}

impl ArrayArgs {
    pub fn new(arg_ptr: u64) -> Self {
        Self { arg_ptr: arg_ptr & 0xFFFF_FFFF, index: 0 }
    }
}

pub trait ArgSource {
    fn get64(&mut self, mem: &dyn GuestMem) -> u64;
    fn get32(&mut self, mem: &dyn GuestMem) -> u32 {
        self.get64(mem) as u32
    }
}

impl ArgSource for StackArgs<'_> {
    fn get64(&mut self, mem: &dyn GuestMem) -> u64 {
        let i = self.index;
        self.index += 1;
        if i <= 7 {
            self.regs.get(i).copied().unwrap_or(0)
        } else {
            let addr = ((self.sp as u32) as u64) + 0x54 + (i as u64 - 8) * 8;
            mem.read_u64(addr)
        }
    }
}

impl ArgSource for ArrayArgs {
    fn get64(&mut self, mem: &dyn GuestMem) -> u64 {
        let addr = self.arg_ptr + 8 * self.index;
        self.index += 1;
        mem.read_u64(addr)
    }
}

// ---- format string input / output ----

struct Input {
    addr: u64,
    wide: bool,
}

impl Input {
    fn at(&self, mem: &dyn GuestMem, offset: u64) -> u16 {
        if self.wide { mem.read_u16(self.addr + offset * 2) } else { mem.read_u8(self.addr + offset) as u16 }
    }
    fn get(&mut self, mem: &dyn GuestMem) -> u16 {
        let c = self.at(mem, 0);
        if c != 0 {
            self.addr += if self.wide { 2 } else { 1 };
        }
        c
    }
    fn peek(&self, mem: &dyn GuestMem, offset: u64) -> u16 {
        self.at(mem, offset)
    }
    fn skip(&mut self, mem: &dyn GuestMem, count: usize) {
        for _ in 0..count {
            if self.get(mem) == 0 {
                break;
            }
        }
    }
}

struct Output {
    /// Narrow output rejects characters >= 0x100 (StringFormatData::put).
    narrow: bool,
    chars: Vec<u16>,
}

impl Output {
    fn put(&mut self, c: u16) -> bool {
        if self.narrow && c >= 0x100 {
            return false;
        }
        self.chars.push(c);
        true
    }
}

// ---- format_core ----

const FF_LEFT_JUSTIFY: u32 = 1 << 0;
const FF_ADD_LEADING_ZEROS: u32 = 1 << 1;
const FF_ADD_POSITIVE: u32 = 1 << 2;
const FF_ADD_POSITIVE_AS_SPACE: u32 = 1 << 3;
const FF_ADD_NEGATIVE: u32 = 1 << 4;
const FF_ADD_PREFIX: u32 = 1 << 5;
const FF_IS_SHORT: u32 = 1 << 6;
const FF_IS_LONG: u32 = 1 << 7;
const FF_IS_LONG_LONG: u32 = 1 << 8;
const FF_IS_WIDE: u32 = 1 << 9;
const FF_IS_SIGNED: u32 = 1 << 10;
const FF_FORCE_LEADING_ZERO: u32 = 1 << 11;
const FF_INVERT_WIDE: u32 = 1 << 12;

const WORK8_LEN: i32 = 512;

#[derive(Clone, Copy, PartialEq)]
enum State {
    Unknown,
    Start,
    Flags,
    Width,
    PrecisionStart,
    Precision,
    Size,
    Type,
}

/// Formats `fmt_addr` (narrow or `wide`) with `args`. Returns the character count, or -1 on error
/// (premature end inside a conversion, or a character >= 0x100 written to narrow output).
fn format_core(mem: &mut dyn GuestMem, fmt_addr: u64, wide: bool, args: &mut dyn ArgSource, out: &mut Output) -> i32 {
    let mut data = Input { addr: fmt_addr & 0xFFFF_FFFF, wide };
    let mut count: i32 = 0;

    let mut state = State::Unknown;
    let mut flags: u32 = 0;
    let mut width: i32 = 0;
    let mut precision: i32 = -1;
    let mut text: Vec<u16> = Vec::new();
    let mut prefix: Vec<u16> = Vec::new();

    let mut c = data.get(mem);
    loop {
        if state == State::Unknown {
            if c == 0 {
                return count;
            } else if c != '%' as u16 {
                if !out.put(c) {
                    return -1;
                }
                count += 1;
                c = data.get(mem);
                continue;
            }
            state = State::Start;
            c = data.get(mem);
        }

        // in any state, if c is \0, it's bad
        if c == 0 {
            return -1;
        }

        // Advance through the conversion states; `continue 'next` consumes `c` and reads the next character.
        let mut emit = false;
        'states: loop {
            match state {
                State::Unknown => unreachable!(),
                State::Start => {
                    if c == '%' as u16 {
                        state = State::Unknown;
                        if !out.put(c) {
                            return -1;
                        }
                        count += 1;
                        break 'states;
                    }
                    state = State::Flags;
                    flags = 0;
                    width = 0;
                    precision = -1;
                    text.clear();
                    prefix.clear();
                }
                State::Flags => {
                    let f = match c as u8 {
                        b'-' => FF_LEFT_JUSTIFY,
                        b'+' => FF_ADD_POSITIVE,
                        b'0' => FF_ADD_LEADING_ZEROS,
                        b' ' => FF_ADD_POSITIVE_AS_SPACE,
                        b'#' => FF_ADD_PREFIX,
                        _ => 0,
                    };
                    if f != 0 && c < 0x100 {
                        flags |= f;
                        break 'states;
                    }
                    state = State::Width;
                }
                State::Width => {
                    if c == '*' as u16 {
                        width = args.get32(mem) as i32;
                        if width < 0 {
                            flags |= FF_LEFT_JUSTIFY;
                            width = width.wrapping_neg();
                        }
                        state = State::PrecisionStart;
                        break 'states;
                    } else if (b'0' as u16..=b'9' as u16).contains(&c) {
                        width = width.wrapping_mul(10).wrapping_add((c - b'0' as u16) as i32);
                        break 'states;
                    }
                    state = State::PrecisionStart;
                }
                State::PrecisionStart => {
                    if c == '.' as u16 {
                        state = State::Precision;
                        precision = 0;
                        break 'states;
                    }
                    state = State::Size;
                }
                State::Precision => {
                    if c == '*' as u16 {
                        precision = args.get32(mem) as i32;
                        if precision < 0 {
                            precision = -1;
                        }
                        state = State::Size;
                        break 'states;
                    } else if (b'0' as u16..=b'9' as u16).contains(&c) {
                        precision = precision.wrapping_mul(10).wrapping_add((c - b'0' as u16) as i32);
                        break 'states;
                    }
                    state = State::Size;
                }
                State::Size => {
                    match c {
                        0x6C /* l */ => {
                            if data.peek(mem, 0) == 'l' as u16 {
                                data.skip(mem, 1);
                                flags |= FF_IS_LONG_LONG;
                            } else {
                                flags |= FF_IS_LONG;
                            }
                        }
                        // 'L' is accepted and ignored (a title uses it instead of 'l'; see Xenia's TODO).
                        0x4C /* L */ => {}
                        0x68 /* h */ => flags |= FF_IS_SHORT,
                        0x77 /* w */ => flags |= FF_IS_WIDE,
                        0x49 /* I */ => {
                            if data.peek(mem, 0) == '6' as u16 && data.peek(mem, 1) == '4' as u16 {
                                data.skip(mem, 2);
                                flags |= FF_IS_LONG_LONG;
                            } else if data.peek(mem, 0) == '3' as u16 && data.peek(mem, 1) == '2' as u16 {
                                data.skip(mem, 2);
                            }
                        }
                        _ => {
                            state = State::Type;
                            continue 'states;
                        }
                    }
                    state = State::Type;
                    break 'states;
                }
                State::Type => {
                    match conversion(mem, c, wide, &mut flags, &mut precision, args, &mut text, &mut prefix, count) {
                        Conv::Text => emit = true,
                        Conv::Consumed => state = State::Unknown,
                    }
                    break 'states;
                }
            }
        }

        if emit {
            if flags & FF_IS_SIGNED != 0 {
                if flags & FF_ADD_NEGATIVE != 0 {
                    prefix = vec!['-' as u16];
                } else if flags & FF_ADD_POSITIVE != 0 {
                    prefix = vec!['+' as u16];
                } else if flags & FF_ADD_POSITIVE_AS_SPACE != 0 {
                    prefix = vec![' ' as u16];
                }
            }

            let mut padding = width - text.len() as i32 - prefix.len() as i32;

            if flags & (FF_LEFT_JUSTIFY | FF_ADD_LEADING_ZEROS) == 0 && padding > 0 {
                count += padding;
                while padding > 0 {
                    if !out.put(' ' as u16) {
                        return -1;
                    }
                    padding -= 1;
                }
            }
            count += prefix.len() as i32;
            for &p in &prefix {
                if !out.put(p) {
                    return -1;
                }
            }
            if flags & FF_ADD_LEADING_ZEROS != 0 && flags & FF_LEFT_JUSTIFY == 0 && padding > 0 {
                count += padding;
                while padding > 0 {
                    if !out.put('0' as u16) {
                        return -1;
                    }
                    padding -= 1;
                }
            }
            for &t in &text {
                if !out.put(t) {
                    return -1;
                }
            }
            count += text.len() as i32;
            if flags & FF_LEFT_JUSTIFY != 0 && padding > 0 {
                count += padding;
                while padding > 0 {
                    if !out.put(' ' as u16) {
                        return -1;
                    }
                    padding -= 1;
                }
            }
            state = State::Unknown;
        }

        c = data.get(mem);
        if state != State::Unknown && c == 0 {
            return -1;
        }
    }
}

enum Conv {
    /// `text`/`prefix` hold the converted field; apply sign, padding and output it.
    Text,
    /// Nothing to output (`%n`).
    Consumed,
}

#[allow(clippy::too_many_arguments)]
fn conversion(
    mem: &mut dyn GuestMem,
    c: u16,
    wide: bool,
    flags: &mut u32,
    precision: &mut i32,
    args: &mut dyn ArgSource,
    text: &mut Vec<u16>,
    prefix: &mut Vec<u16>,
    count: i32,
) -> Conv {
    let ch = if c < 0x80 { c as u8 as char } else { '\u{0}' };
    match ch {
        'C' | 'c' => {
            if ch == 'C' {
                *flags |= FF_INVERT_WIDE;
            }
            let is_wide = if *flags & (FF_IS_LONG | FF_IS_WIDE) != 0 {
                true
            } else if *flags & FF_IS_SHORT != 0 {
                false
            } else {
                ((*flags & FF_INVERT_WIDE) != 0) ^ wide
            };
            let value = args.get32(mem);
            text.push(if is_wide { value as u16 } else { value as u8 as u16 });
        }
        'd' | 'i' => {
            *flags |= FF_IS_SIGNED;
            integer(mem, b"0123456789", flags, precision, args, text, prefix);
        }
        'o' => {
            if *flags & FF_ADD_PREFIX != 0 {
                *flags |= FF_FORCE_LEADING_ZERO;
            }
            integer(mem, b"01234567", flags, precision, args, text, prefix);
        }
        'u' => integer(mem, b"0123456789", flags, precision, args, text, prefix),
        'x' | 'X' => {
            if *flags & FF_ADD_PREFIX != 0 {
                *prefix = vec!['0' as u16, c];
            }
            let digits: &[u8; 16] = if ch == 'x' { b"0123456789abcdef" } else { b"0123456789ABCDEF" };
            integer(mem, digits, flags, precision, args, text, prefix);
        }
        'e' | 'E' | 'f' | 'g' | 'G' | 'a' | 'A' => {
            *flags |= FF_IS_SIGNED;
            let mut value = f64::from_bits(args.get64(mem));
            if value < 0.0 {
                value = -value;
                *flags |= FF_ADD_NEGATIVE;
            }
            let s = format_double(value, *precision, ch, *flags);
            text.extend(s.bytes().map(|b| b as u16));
        }
        'n' => {
            let pointer = args.get32(mem) as u64;
            if *flags & FF_IS_SHORT != 0 {
                mem.write_u16(pointer, count as u16);
            } else {
                mem.write_u32(pointer, count as u32);
            }
            return Conv::Consumed;
        }
        'p' => {
            *precision = 8;
            *flags &= !(FF_IS_LONG_LONG | FF_IS_SHORT);
            *flags |= FF_IS_LONG;
            integer(mem, b"0123456789ABCDEF", flags, precision, args, text, prefix);
        }
        'S' | 's' => {
            if ch == 'S' {
                *flags |= FF_INVERT_WIDE;
            }
            let pointer = args.get32(mem) as u64;
            let mut cap = if *precision < 0 { i32::MAX } else { *precision };
            if pointer == 0 {
                text.extend("(null)".bytes().take(cap.max(0) as usize).map(|b| b as u16));
            } else {
                let is_wide = if *flags & (FF_IS_LONG | FF_IS_WIDE) != 0 {
                    true
                } else if *flags & FF_IS_SHORT != 0 {
                    false
                } else {
                    ((*flags & FF_INVERT_WIDE) != 0) ^ wide
                };
                let mut a = pointer;
                while cap > 0 {
                    let v = if is_wide { mem.read_u16(a) } else { mem.read_u8(a) as u16 };
                    if v == 0 {
                        break;
                    }
                    text.push(v);
                    a += if is_wide { 2 } else { 1 };
                    cap -= 1;
                }
            }
        }
        // 'Z' (ANSI_STRING/UNICODE_STRING) and unknown types: Xenia asserts and emits an empty field.
        _ => {}
    }
    Conv::Text
}

fn integer(
    mem: &mut dyn GuestMem,
    digits: &[u8],
    flags: &mut u32,
    precision: &mut i32,
    args: &mut dyn ArgSource,
    text: &mut Vec<u16>,
    prefix: &mut Vec<u16>,
) {
    let radix = digits.len() as u64;
    let mut value: i64 = if *flags & FF_IS_LONG_LONG != 0 {
        args.get64(mem) as i64
    } else if *flags & FF_IS_SHORT != 0 {
        // Deviation from Xenia: unsigned short conversions zero-extend (%hu of 0xFFFF is 65535, as in MSVC);
        // Xenia sign-extends every 'h' value and prints 4294967295.
        let v = args.get32(mem) as u16;
        if *flags & FF_IS_SIGNED != 0 { v as i16 as i64 } else { v as i64 }
    } else {
        args.get32(mem) as i32 as i64
    };

    if *precision >= 0 {
        *precision = (*precision).min(WORK8_LEN);
    } else {
        *precision = 1;
    }

    if *flags & FF_IS_SIGNED != 0 && value < 0 {
        value = value.wrapping_neg();
        *flags |= FF_ADD_NEGATIVE;
    }
    if *flags & FF_IS_LONG_LONG == 0 {
        value &= u32::MAX as i64;
    }
    if value == 0 {
        prefix.clear();
    }

    // Deviation from Xenia: digits are produced with unsigned arithmetic, so %llx/%llu of values >= 2^63 are
    // correct (Xenia divides the signed value and truncates those).
    let mut v = value as u64;
    let mut out: Vec<u16> = Vec::new();
    let mut p = *precision;
    while p > 0 || v != 0 {
        out.push(digits[(v % radix) as usize] as u16);
        v /= radix;
        p -= 1;
    }
    if *flags & FF_FORCE_LEADING_ZERO != 0 && (out.is_empty() || *out.last().unwrap() != '0' as u16) {
        out.push('0' as u16);
    }
    out.reverse();
    *text = out;
}

/// std::ostringstream formatting of Xenia's format_double (setprecision + fixed/scientific/hexfloat/defaultfloat,
/// uppercase for E/G/A, showpoint for '#'). `value` is non-negative; the sign is added by the caller.
fn format_double(value: f64, precision: i32, c: char, flags: u32) -> String {
    let mut precision = precision;
    if precision < 0 {
        precision = 6;
    } else if precision == 0 && c == 'g' {
        precision = 1;
    }
    let p = precision as usize;
    let upper = matches!(c, 'E' | 'G' | 'A');
    let showpoint = flags & FF_ADD_PREFIX != 0;

    let s = if value.is_nan() {
        "nan".to_string()
    } else if value.is_infinite() {
        "inf".to_string()
    } else {
        match c {
            'f' => {
                let mut s = format!("{:.*}", p, value);
                if showpoint && p == 0 {
                    s.push('.');
                }
                s
            }
            'e' | 'E' => sci(value, p, showpoint),
            'a' | 'A' => hexfloat(value),
            // 'g' / 'G': defaultfloat. iostream uses %g semantics but 'G' with precision 0 keeps 0, which %g
            // treats as 1 (Xenia only bumps lowercase 'g').
            _ => general(value, p.max(1), showpoint),
        }
    };
    if upper { s.to_uppercase() } else { s }
}

/// %e: d.ddde±XX (at least two exponent digits).
fn sci(value: f64, p: usize, showpoint: bool) -> String {
    let s = format!("{:.*e}", p, value);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let mut mant = mant.to_string();
    if showpoint && p == 0 {
        mant.push('.');
    }
    format!("{}e{}{:02}", mant, if exp < 0 { '-' } else { '+' }, exp.abs())
}

/// %g with `p` significant digits.
fn general(value: f64, p: usize, showpoint: bool) -> String {
    if value == 0.0 {
        return if showpoint { format!("{:.*}", p - 1, 0.0) } else { "0".to_string() };
    }
    let exp: i32 = format!("{:.*e}", p - 1, value).split_once('e').unwrap().1.parse().unwrap();
    let mut s = if exp < -4 || exp >= p as i32 {
        sci(value, p - 1, showpoint)
    } else {
        format!("{:.*}", (p as i32 - 1 - exp).max(0) as usize, value)
    };
    if !showpoint {
        // strip trailing zeros of the fraction (and a dangling '.')
        let (num, tail) = match s.find('e') {
            Some(i) => (s[..i].to_string(), s[i..].to_string()),
            None => (s.clone(), String::new()),
        };
        let num = if num.contains('.') { num.trim_end_matches('0').trim_end_matches('.').to_string() } else { num };
        s = num + &tail;
    }
    s
}

/// std::hexfloat: 0x1.hhhp±d (normalized; subnormals as 0x0.hhhp-1022).
fn hexfloat(value: f64) -> String {
    if value == 0.0 {
        return "0x0p+0".to_string();
    }
    let bits = value.to_bits();
    let exp_bits = ((bits >> 52) & 0x7FF) as i32;
    let mut mant = bits & ((1u64 << 52) - 1);
    let (lead, exp) = if exp_bits == 0 { (0, -1022) } else { (1, exp_bits - 1023) };
    let mut hex = String::new();
    let mut digits = 13;
    while digits > 0 && mant & 0xF == 0 {
        mant >>= 4;
        digits -= 1;
    }
    if digits > 0 {
        hex = format!(".{:0width$x}", mant, width = digits);
    }
    format!("0x{}{}p{}{}", lead, hex, if exp < 0 { '-' } else { '+' }, exp.abs())
}

// ---- exports ----

fn write_narrow(mem: &mut dyn GuestMem, buf: u64, chars: &[u16]) {
    for (i, &c) in chars.iter().enumerate() {
        mem.write_u8(buf + i as u64, c as u8);
    }
}

fn write_wide(mem: &mut dyn GuestMem, buf: u64, chars: &[u16]) {
    for (i, &c) in chars.iter().enumerate() {
        mem.write_u16(buf + 2 * i as u64, c);
    }
}

fn ret(count: i32) -> u64 {
    count as u32 as u64
}

/// _snprintf / _vsnprintf (narrow) and _snwprintf / _vsnwprintf (wide), bounded by `buffer_count` characters.
/// Overflow returns -1, except `_vsnwprintf`, which returns the full count (`overflow_returns_count`).
pub fn snprintf(mem: &mut dyn GuestMem, buf: u64, buffer_count: u32, fmt: u64, args: &mut dyn ArgSource, wide: bool, overflow_returns_count: bool) -> u64 {
    let buf = buf & 0xFFFF_FFFF;
    if buf == 0 || buffer_count == 0 || fmt & 0xFFFF_FFFF == 0 {
        return ret(-1);
    }
    let mut out = Output { narrow: !wide, chars: Vec::new() };
    let mut count = format_core(mem, fmt, wide, args, &mut out);
    let write = |mem: &mut dyn GuestMem, chars: &[u16]| if wide { write_wide(mem, buf, chars) } else { write_narrow(mem, buf, chars) };
    if count < 0 {
        write(mem, &[0]);
    } else if count as u32 <= buffer_count {
        write(mem, &out.chars[..count as usize]);
        if (count as u32) < buffer_count {
            let unit = if wide { 2 } else { 1 };
            write_at(mem, buf + count as u64 * unit, wide);
        }
    } else {
        write(mem, &out.chars[..buffer_count as usize]);
        if !overflow_returns_count {
            count = -1;
        }
    }
    ret(count)
}

fn write_at(mem: &mut dyn GuestMem, addr: u64, wide: bool) {
    if wide { mem.write_u16(addr, 0) } else { mem.write_u8(addr, 0) }
}

/// sprintf / vsprintf (narrow) and swprintf / vswprintf (wide): unbounded, always NUL-terminated.
pub fn sprintf(mem: &mut dyn GuestMem, buf: u64, fmt: u64, args: &mut dyn ArgSource, wide: bool) -> u64 {
    let buf = buf & 0xFFFF_FFFF;
    if buf == 0 || fmt & 0xFFFF_FFFF == 0 {
        return ret(-1);
    }
    let mut out = Output { narrow: !wide, chars: Vec::new() };
    let count = format_core(mem, fmt, wide, args, &mut out);
    if count <= 0 {
        write_at(mem, buf, wide);
    } else {
        let unit = if wide { 2 } else { 1 };
        if wide { write_wide(mem, buf, &out.chars) } else { write_narrow(mem, buf, &out.chars) }
        write_at(mem, buf + count as u64 * unit, wide);
    }
    ret(count)
}

/// _vscwprintf: number of characters the wide format would produce.
pub fn vscwprintf(mem: &mut dyn GuestMem, fmt: u64, args: &mut dyn ArgSource) -> u64 {
    if fmt & 0xFFFF_FFFF == 0 {
        return ret(-1);
    }
    let mut out = Output { narrow: false, chars: Vec::new() };
    ret(format_core(mem, fmt, true, args, &mut out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;

    type Mem = FakeMem;

    const FMT: u64 = 0x1000;
    const BUF: u64 = 0x2000;
    const S1: u64 = 0x3000;
    const S2: u64 = 0x3100;
    const S3: u64 = 0x3200;

    /// Narrow sprintf with register arguments (index 2 onwards, like Xenia's StackArgList(ctx, 2)).
    fn fmt_n(mem: &mut Mem, fmt: &str, args: &[u64]) -> (String, i32) {
        mem.put_str(FMT, fmt);
        let regs: Vec<u64> = [BUF, FMT].iter().copied().chain(args.iter().copied()).collect();
        let r = sprintf(mem, BUF, FMT, &mut StackArgs::new(&regs, 0, 2), false) as u32 as i32;
        (mem.get_str(BUF), r)
    }

    fn fmt_w(mem: &mut Mem, fmt: &str, args: &[u64]) -> (String, i32) {
        mem.put_wstr(FMT, fmt);
        let regs: Vec<u64> = [BUF, FMT].iter().copied().chain(args.iter().copied()).collect();
        let r = sprintf(mem, BUF, FMT, &mut StackArgs::new(&regs, 0, 2), true) as u32 as i32;
        (mem.get_wstr(BUF), r)
    }

    fn n(fmt: &str, args: &[u64]) -> String { fmt_n(&mut Mem::default(), fmt, args).0 }

    #[test]
    fn integers() {
        assert_eq!(n("%d|%i|%u", &[(-42i32) as u32 as u64, 7, 0xFFFF_FFFF]), "-42|7|4294967295");
        assert_eq!(n("%x %X %#x %#X %#x", &[0xBEEF, 0xBEEF, 0x1A, 0x1A, 0]), "beef BEEF 0x1a 0X1A 0");
        assert_eq!(n("%o %#o", &[8, 8]), "10 010");
        assert_eq!(n("%5d|%-5d|%05d|%+d|% d", &[42, 42, (-42i32) as u32 as u64, 5, 5]), "   42|42   |-0042|+5| 5");
        // '0' pads with zeros even with a precision (Xenia; MSVC would ignore '0' here)
        assert_eq!(n("%.3d|%.0d|%08.3x", &[5, 0, 0xA]), "005||0000000a");
        assert_eq!(n("%hd %hu", &[0x1_FFFF, 0x1_FFFF]), "-1 65535");
        assert_eq!(n("%lld %llx %I64u", &[(-5i64) as u64, 0xFFFF_FFFF_FFFF_FFFF, 1 << 40]), "-5 ffffffffffffffff 1099511627776");
        assert_eq!(n("%I32d %ld", &[(-3i32) as u32 as u64, 9]), "-3 9");
        assert_eq!(n("%p", &[0x1234]), "00001234");
        assert_eq!(n("%*d|%-*d|%.*d", &[4, 1, (-4i32) as u32 as u64, 1, 3, 2]), "   1|1   |002");
    }

    #[test]
    fn strings_and_chars() {
        let mut m = Mem::default();
        m.put_str(S1, "abc");
        m.put_wstr(S2, "wide");
        assert_eq!(fmt_n(&mut m, "[%s][%.2s][%5s][%-5s][%S][%ls]", &[S1, S1, S1, S1, S2, S2]).0, "[abc][ab][  abc][abc  ][wide][wide]");
        assert_eq!(n("%s|%.3s", &[0, 0]), "(null)|(nu");
        assert_eq!(n("%c%c%%", &[b'h' as u64, b'i' as u64]), "hi%");
    }

    #[test]
    fn wide_functions_swap_s_meaning() {
        let mut m = Mem::default();
        m.put_str(S1, "narrow");
        m.put_wstr(S2, "wide");
        assert_eq!(fmt_w(&mut m, "%s/%S/%hs/%ls/%c/%C", &[S2, S1, S1, S2, 'W' as u64, b'n' as u64]).0, "wide/narrow/narrow/wide/W/n");
    }

    /// The dash's XUI locator (0x92187BE0): _snwprintf(buf, 128, L"section://%X,%s%s%s", hModule, name, L"#", L"skin.xur").
    #[test]
    fn dash_skin_locator() {
        let mut m = Mem::default();
        m.put_wstr(FMT, "section://%X,%s%s%s");
        m.put_wstr(S1, "DASHUISKIN");
        m.put_wstr(S2, "#");
        m.put_wstr(S3, "skin.xur");
        let regs = [BUF, 128, FMT, 0x9200_0000, S1, S2, S3];
        let r = snprintf(&mut m, BUF, 128, FMT, &mut StackArgs::new(&regs, 0, 3), true, false);
        assert_eq!(m.get_wstr(BUF), "section://92000000,DASHUISKIN#skin.xur");
        assert_eq!(r, 38);
    }

    #[test]
    fn snprintf_bounds() {
        let mut m = Mem::default();
        m.put_str(FMT, "hello");
        for i in 0..8 { m.write_u8(BUF + i, 0xEE); }
        let regs = [BUF, 3, FMT];
        let r = snprintf(&mut m, BUF, 3, FMT, &mut StackArgs::new(&regs, 0, 3), false, false);
        assert_eq!(r as u32 as i32, -1);
        assert_eq!((m.read_u8(BUF), m.read_u8(BUF + 2), m.read_u8(BUF + 3)), (b'h', b'l', 0xEE)); // no NUL on overflow
        let r = snprintf(&mut m, BUF, 5, FMT, &mut StackArgs::new(&regs, 0, 3), false, false);
        assert_eq!((r, m.read_u8(BUF + 4), m.read_u8(BUF + 5)), (5, b'o', 0xEE)); // exact fit: no NUL either
        let r = snprintf(&mut m, BUF, 6, FMT, &mut StackArgs::new(&regs, 0, 3), false, false);
        assert_eq!((r, m.get_str(BUF)), (5, "hello".to_string()));
        assert_eq!(snprintf(&mut m, BUF, 0, FMT, &mut StackArgs::new(&regs, 0, 3), false, false) as u32 as i32, -1);
        // _vsnwprintf keeps the full count on overflow
        m.put_wstr(FMT, "hello");
        assert_eq!(snprintf(&mut m, BUF, 2, FMT, &mut ArrayArgs::new(0), true, true), 5);
    }

    #[test]
    fn errors() {
        // narrow output rejects characters >= 0x100
        let mut m = Mem::default();
        m.put_wstr(S2, "\u{263A}");
        let (s, r) = fmt_n(&mut m, "x%ls", &[S2]);
        assert_eq!((s, r), (String::new(), -1));
        // format ending inside a conversion
        assert_eq!(fmt_n(&mut Mem::default(), "abc%", &[]).1, -1);
        assert_eq!(fmt_n(&mut Mem::default(), "%-", &[]).1, -1);
    }

    #[test]
    fn stack_and_va_list_arguments() {
        let mut m = Mem::default();
        // 10 integers: r3..r10 hold buf, fmt and the first 6; the rest at r1 + 0x54 + 8*i (u64 big-endian)
        let sp = 0x8000;
        for i in 0..4u64 {
            m.write_u32(sp + 0x54 + 8 * i, 0);
            m.write_u32(sp + 0x58 + 8 * i, 7 + i as u32);
        }
        m.put_str(FMT, "%d%d%d%d%d%d%d%d%d%d");
        let regs = [BUF, FMT, 1, 2, 3, 4, 5, 6];
        sprintf(&mut m, BUF, FMT, &mut StackArgs::new(&regs, sp, 2), false);
        assert_eq!(m.get_str(BUF), "12345678910");
        // va_list: 64-bit slots
        let va = 0x9000;
        for (i, v) in [0xABu64, (-1i64) as u64].iter().enumerate() {
            m.write_u32(va + 8 * i as u64, (v >> 32) as u32);
            m.write_u32(va + 8 * i as u64 + 4, *v as u32);
        }
        m.put_wstr(FMT, "%x,%d");
        assert_eq!(vscwprintf(&mut m, FMT, &mut ArrayArgs::new(va)), 5);
        sprintf(&mut m, BUF, FMT, &mut ArrayArgs::new(va), true);
        assert_eq!(m.get_wstr(BUF), "ab,-1");
    }

    #[test]
    fn percent_n() {
        let mut m = Mem::default();
        let (s, r) = fmt_n(&mut m, "abc%n!", &[0x5000]);
        assert_eq!((s.as_str(), r, m.read_u32(0x5000)), ("abc!", 4, 3));
    }

    #[test]
    fn floats() {
        let f = |v: f64| v.to_bits();
        assert_eq!(n("%f|%.2f|%.0f|%#.0f", &[f(3.5), f(-2.345), f(2.5), f(3.0)]), "3.500000|-2.35|2|3.");
        assert_eq!(n("%e|%.2E|%e", &[f(1234.5), f(0.000123), f(0.0)]), "1.234500e+03|1.23E-04|0.000000e+00");
        assert_eq!(n("%g|%g|%g|%G|%.3g", &[f(100000.0), f(1e6), f(0.0001), f(1e-5), f(3.14159)]), "100000|1e+06|0.0001|1E-05|3.14");
        assert_eq!(n("%a", &[f(1.5)]), "0x1.8p+0");
        assert_eq!(n("%8.3f|%-8.3f|%08.3f", &[f(1.5), f(1.5), f(-1.5)]), "   1.500|1.500   |-001.500");
    }
}
