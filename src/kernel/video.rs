// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/kernel.h                  struct X_VIDEO_MODE (48 bytes)
//   src/xenia/kernel/xboxkrnl/xboxkrnl_video.cc VdQueryVideoMode, GetVideoRefreshRate, GetVideoStandard
//   src/xenia/kernel/xconfig.h                 X_AV_VIDEO_STANDARD
//   src/xenia/kernel/xboxkrnl/xboxkrnl_video.cc VdSwap; src/xenia/gpu/xenos.h MakePacketType0/2/3,
//   xe_gpu_texture_fetch_t, kSwapSignature; src/xenia/gpu/register_table.inc SHADER_CONSTANT_FETCH_00_0
//
//! Video mode reported to the guest. Xenia derives it from XConfig (screen size, video flags, AV region); we
//! keep the equivalent settings in `VideoConfig`, defaulting to 1280x720 progressive 16:9 NTSC-M at 60 Hz.

use super::guest_mem::GuestMem;
use super::strings::{ArgSource, StackArgs};

/// X_AV_VIDEO_STANDARD
pub const VIDEO_STANDARD_NTSC_M: u32 = 1;
pub const VIDEO_STANDARD_NTSC_J: u32 = 2;
pub const VIDEO_STANDARD_PAL: u32 = 3;

pub const X_VIDEO_MODE_SIZE: u64 = 48;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoConfig {
    pub width: u32,
    pub height: u32,
    pub interlaced: bool,
    pub widescreen: bool,
    /// 50 Hz (PAL-50 region); 60 Hz otherwise.
    pub refresh_50hz: bool,
    /// X_AV_VIDEO_STANDARD (1..3); PAL-50 is reported as PAL with `refresh_50hz`.
    pub video_standard: u32,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self { width: 1280, height: 720, interlaced: false, widescreen: true, refresh_50hz: false, video_standard: VIDEO_STANDARD_NTSC_M }
    }
}

impl VideoConfig {
    /// VdQueryVideoMode: zero the X_VIDEO_MODE and fill it.
    pub fn write_video_mode(&self, mem: &mut dyn GuestMem, ptr: u64) {
        let p = ptr & 0xFFFF_FFFF;
        if p == 0 {
            return;
        }
        for off in (0..X_VIDEO_MODE_SIZE).step_by(4) {
            mem.write_u32(p + off, 0);
        }
        mem.write_u32(p, self.width); // display_width
        mem.write_u32(p + 0x04, self.height); // display_height
        mem.write_u32(p + 0x08, self.interlaced as u32); // is_interlaced
        mem.write_u32(p + 0x0C, self.widescreen as u32); // is_widescreen
        mem.write_u32(p + 0x10, (self.width >= 0x500) as u32); // is_hi_def
        let refresh: f32 = if self.refresh_50hz { 50.0 } else { 60.0 };
        mem.write_u32(p + 0x14, refresh.to_bits()); // refresh_rate (float)
        mem.write_u32(p + 0x18, self.video_standard); // video_standard
        mem.write_u32(p + 0x1C, 0x8A); // pixel_rate
        mem.write_u32(p + 0x20, if self.widescreen { 0x01 } else { 0x03 }); // widescreen_flag
        // 0x24..0x30: reserved[3] = 0
    }
}

/// SHADER_CONSTANT_FETCH_00_0 (register_table.inc).
pub const REG_SHADER_CONSTANT_FETCH_00_0: u32 = 0x4800;
/// PM4 type-3 XE_SWAP (Xenia-specific swap token, xenos.h).
pub const PM4_XE_SWAP: u32 = 0x64;
/// kSwapSignature = make_fourcc("SWAP"), packed big-endian (0x53 = 'S' in the high byte). Written by VdSwap and
/// checked by the CP's XE_SWAP handler, so both sides use this value.
pub const SWAP_SIGNATURE: u32 = 0x5357_4150;
/// Words the D3D runtime reserves in the command buffer for VdSwap.
pub const VD_SWAP_BUFFER_DWORDS: u64 = 64;

/// xenos::MakePacketType0(index, count): write `count` consecutive registers starting at `index`.
pub fn make_packet_type0(index: u32, count: u32) -> u32 {
    (((count - 1) & 0x3FFF) << 16) | (index & 0x7FFF)
}
/// xenos::MakePacketType2(): one-dword NOP.
pub fn make_packet_type2() -> u32 {
    2 << 30
}
/// xenos::MakePacketType3(opcode, count) without predication.
pub fn make_packet_type3(opcode: u32, count: u32) -> u32 {
    (3 << 30) | (((count - 1) & 0x3FFF) << 16) | ((opcode & 0x7F) << 8)
}

/// Guest virtual -> GPU physical address. In this emulator the physical-memory aliases (MmAllocatePhysicalMemoryEx,
/// 0x9xxxxxxx) map to physical by dropping the top 3 bits, the same mapping the D3D uses for the ring and fences
/// (virtual 0x90102000 is written by the GPU as 0x10102000). Xenia uses Memory::GetPhysicalAddress.
pub fn gpu_physical_address(virtual_address: u32) -> u32 {
    virtual_address & 0x1FFF_FFFF
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwapInfo {
    /// Frontbuffer guest virtual address (*frontbuffer_ptr).
    pub frontbuffer: u32,
    pub frontbuffer_physical: u32,
    pub width: u32,
    pub height: u32,
}

/// VdSwap(buffer_ptr, fetch_ptr, unk2, unk3, unk4, frontbuffer_ptr, texture_format_ptr, color_space_ptr, width_ptr,
/// height_ptr): fills the 64 words the caller reserved in its command buffer with the frontbuffer texture fetch
/// (base address rewritten as physical), an XE_SWAP packet and type-2 NOPs, like Xenia's VdSwap_entry.
/// `regs` = r3..r10; width/height pointers are stack arguments 8 and 9. `None` if a required pointer is NULL.
pub fn vd_swap(mem: &mut dyn GuestMem, regs: &[u64], sp: u64) -> Option<SwapInfo> {
    let mut args = StackArgs::new(regs, sp, 0);
    let a: Vec<u64> = (0..10).map(|_| args.get32(mem) as u64).collect();
    let (buffer_ptr, fetch_ptr, frontbuffer_ptr, width_ptr, height_ptr) = (a[0], a[1], a[5], a[8], a[9]);
    if buffer_ptr == 0 || fetch_ptr == 0 || frontbuffer_ptr == 0 || width_ptr == 0 || height_ptr == 0 {
        return None;
    }
    let mut fetch: Vec<u32> = (0..6).map(|i| mem.read_u32(fetch_ptr + 4 * i)).collect();
    let frontbuffer_virtual = fetch[1] & 0xFFFF_F000; // base_address (bits 12..31 of dword_1) << 12
    let frontbuffer_physical = gpu_physical_address(frontbuffer_virtual);
    fetch[1] = (fetch[1] & 0xFFF) | (frontbuffer_physical & 0xFFFF_F000);
    let width = mem.read_u32(width_ptr);
    let height = mem.read_u32(height_ptr);

    let mut words: Vec<u32> = Vec::with_capacity(VD_SWAP_BUFFER_DWORDS as usize);
    words.push(make_packet_type0(REG_SHADER_CONSTANT_FETCH_00_0, 6));
    words.extend_from_slice(&fetch);
    words.push(make_packet_type3(PM4_XE_SWAP, 4));
    words.extend_from_slice(&[SWAP_SIGNATURE, frontbuffer_physical, width, height]);
    while (words.len() as u64) < VD_SWAP_BUFFER_DWORDS {
        words.push(make_packet_type2());
    }
    for (i, w) in words.iter().enumerate() {
        mem.write_u32(buffer_ptr + 4 * i as u64, *w);
    }
    Some(SwapInfo { frontbuffer: mem.read_u32(frontbuffer_ptr), frontbuffer_physical, width, height })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::guest_mem::fake::FakeMem;

    const P: u64 = 0x9301_7E70; // like the dash's stack buffer at r1+0x60

    #[test]
    fn default_mode_layout() {
        let mut m = FakeMem::default();
        for off in (0..0x40).step_by(4) {
            m.write_u32(P + off, 0xDEAD_BEEF);
        }
        VideoConfig::default().write_video_mode(&mut m, P);
        let w = |o: u64| m.read_u32(P + o);
        assert_eq!((w(0x00), w(0x04), w(0x08), w(0x0C), w(0x10)), (1280, 720, 0, 1, 1));
        assert_eq!(f32::from_bits(w(0x14)), 60.0);
        assert_eq!((w(0x18), w(0x1C), w(0x20)), (VIDEO_STANDARD_NTSC_M, 0x8A, 1));
        assert_eq!((w(0x24), w(0x28), w(0x2C)), (0, 0, 0));
        // the structure is exactly 48 bytes: nothing past it is touched
        assert_eq!(w(0x30), 0xDEAD_BEEF);
    }

    #[test]
    fn non_widescreen_pal50_sd() {
        let mut m = FakeMem::default();
        let cfg = VideoConfig { width: 640, height: 480, interlaced: true, widescreen: false, refresh_50hz: true, video_standard: VIDEO_STANDARD_PAL };
        cfg.write_video_mode(&mut m, P);
        let w = |o: u64| m.read_u32(P + o);
        assert_eq!((w(0x08), w(0x0C), w(0x10)), (1, 0, 0));
        assert_eq!(f32::from_bits(w(0x14)), 50.0);
        assert_eq!((w(0x18), w(0x20)), (VIDEO_STANDARD_PAL, 3));
    }

    #[test]
    fn null_pointer_is_ignored() {
        let mut m = FakeMem::default();
        VideoConfig::default().write_video_mode(&mut m, 0);
        assert_eq!(m.read_u32(0), 0);
    }

    #[test]
    fn vd_swap_writes_fetch_swap_packet_and_nops() {
        let mut m = FakeMem::default();
        const BUF: u64 = 0x9010_C640; // the dash's buffer_ptr (inside its command buffer)
        const FETCH: u64 = 0x9301_7AA0;
        const FB: u64 = 0x9301_7A8C;
        const SP: u64 = 0x9301_7A20;
        for i in 0..70 {
            m.write_u32(BUF + 4 * i, 0xEEEE_EEEE);
        }
        // fetch constant with base_address = 0x90314000 >> 12 in dword_1 (format bits kept)
        let fetch = [0x1000_0002, 0x9031_4086, 0x0059_E4FF, 0x0000_0D10, 0x0000_0000, 0x0000_0200];
        for (i, w) in fetch.iter().enumerate() {
            m.write_u32(FETCH + 4 * i as u64, *w);
        }
        m.write_u32(FB, 0x9031_4000);
        // width/height pointers are stack arguments 8 and 9: low words of the 8-byte slots at r1+0x50 and r1+0x58
        // (layout observed at the dash's VdSwap call: r1+0x50 = 00000000 93017A80 00000000 93017A84)
        m.write_u32(SP + 0x54, 0x9301_7B00);
        m.write_u32(SP + 0x5C, 0x9301_7B04);
        m.write_u32(0x9301_7B00, 1280);
        m.write_u32(0x9301_7B04, 720);
        let regs = [BUF, FETCH, 0x9010_2008, 0x9301_7AF0, 0, FB, 0x9301_7A90, 0x9301_7A88];
        let info = vd_swap(&mut m, &regs, SP).unwrap();
        assert_eq!(info, SwapInfo { frontbuffer: 0x9031_4000, frontbuffer_physical: 0x1031_4000, width: 1280, height: 720 });
        let w = |i: u64| m.read_u32(BUF + 4 * i);
        assert_eq!(w(0), 0x0005_4800); // type 0: 6 registers from SHADER_CONSTANT_FETCH_00_0
        assert_eq!((w(1), w(2), w(6)), (0x1000_0002, 0x1031_4086, 0x0000_0200)); // base rewritten as physical
        assert_eq!(w(7), 0xC003_6400); // type 3 XE_SWAP, 4 data words
        assert_eq!((w(8), w(9), w(10), w(11)), (SWAP_SIGNATURE, 0x1031_4000, 1280, 720));
        assert!((12..64).all(|i| w(i) == 0x8000_0000));
        assert_eq!(w(64), 0xEEEE_EEEE, "exactly 64 words");
    }

    #[test]
    fn vd_swap_requires_pointers() {
        let mut m = FakeMem::default();
        assert_eq!(vd_swap(&mut m, &[0, 1, 0, 0, 0, 1, 0, 0], 0x1000), None);
    }
}
