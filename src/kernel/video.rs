// Derived from Xenia (BSD-3-Clause), Copyright (c) 2015 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/kernel.h                  struct X_VIDEO_MODE (48 bytes)
//   src/xenia/kernel/xboxkrnl/xboxkrnl_video.cc VdQueryVideoMode, GetVideoRefreshRate, GetVideoStandard
//   src/xenia/kernel/xconfig.h                 X_AV_VIDEO_STANDARD
//
//! Video mode reported to the guest. Xenia derives it from XConfig (screen size, video flags, AV region); we
//! keep the equivalent settings in `VideoConfig`, defaulting to 1280x720 progressive 16:9 NTSC-M at 60 Hz.

use super::guest_mem::GuestMem;

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
}
