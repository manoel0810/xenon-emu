// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xconfig.cc          XConfig::SetDefaults, ReadSetting, WriteSetting, GetSettingSize
//   src/xenia/kernel/xconfig.h           XConfigData (category blocks), kFields, X_CONFIG_CATEGORY, setting ids,
//                                        X_AV_REGION, X_AUDIO_FLAGS, X_RETAIL_FLAGS, X_PC_FLAGS, X_PC_GAMES_FLAGS,
//                                        XHDTVResolution, XVGAResolution, kTimezones
//   src/xenia/kernel/xam/xam_locale.cc   xeXamGetLocaleFromCountry, xeXamGetLocaleEx, xeGetLanguage
//   src/xenia/xbox.h                     XLanguage, XOnlineCountry
//
//! The console configuration (XConfig) at Xenia's defaults: one big-endian block per category, read and written
//! through Xenia's field table (ExGetXConfigSetting / ExSetXConfigSetting). Not persisted to a file as Xenia does.

/// XCONFIG_USER_LANGUAGE: XLanguage::kEnglish.
pub const USER_LANGUAGE: u32 = 1;
/// XCONFIG_USER_COUNTRY: XOnlineCountry::kUnitedStates.
pub const USER_COUNTRY: u8 = 103;
/// XLanguage::kMaxLanguages (extended languages).
const MAX_LANGUAGES: u32 = 18;

/// xeXamGetLocaleFromCountry.
const LOCALE_FROM_COUNTRY: [u8; 112] = [
    0, 43, 0, 0, 40, 2, 1, 0, 3, 0, 0, 0, 0, 4, 0, 0, 5, 0, 33,
    6, 7, 8, 0, 9, 13, 10, 0, 0, 0, 0, 0, 31, 11, 0, 12, 35, 0, 14,
    0, 15, 0, 0, 16, 0, 18, 42, 17, 0, 0, 0, 19, 0, 0, 20, 0, 0, 21,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 22, 0, 0, 23, 25,
    24, 0, 0, 0, 0, 0, 26, 0, 27, 0, 0, 0, 37, 41, 32, 28, 0, 29, 0,
    0, 0, 0, 0, 39, 0, 34, 0, 36, 0, 0, 0, 0, 0, 30, 0, 38,
];

/// xeGetLanguage(extended languages): the configured language when valid; English outside NTSC-J (the dash's
/// region is not Japan/Korea here).
pub fn language() -> u32 {
    if USER_LANGUAGE < MAX_LANGUAGES { USER_LANGUAGE } else { 1 }
}

/// xeXamGetLocale (xeXamGetLocaleEx(111, 43)): the configured country's locale, else the region fallback (US = 36).
pub fn locale() -> u8 {
    let country = USER_COUNTRY;
    if country <= 111 {
        let locale = LOCALE_FROM_COUNTRY.get(country as usize).copied().unwrap_or(0);
        if locale <= 43 {
            return locale;
        }
    }
    36
}

pub const CATEGORY_SECURED: u16 = 0x02;
pub const CATEGORY_USER: u16 = 0x03;
pub const CATEGORY_CONSOLE: u16 = 0x07;
pub const CATEGORY_IPTV: u16 = 0x09;
pub const CATEGORY_SYSTEM: u16 = 0x0A;
pub const USER_RETAIL_FLAGS: u16 = 0x0C;

/// Byte size of each category's block in XConfigData, by X_CONFIG_CATEGORY (static .. system).
const CATEGORY_SIZES: [usize; 11] = [0x10E, 0x5D8, 0x200, 0x1FD, 0x1F0, 0x0D, 0x74C, 0x144, 0x298, 0x200, 0x10];

/// kFields: (category, setting, offset in the category block, size).
const FIELDS: &[(u16, u16, usize, u16)] = &[
    // XCONFIG_STATIC: first_power_on
    (0x00, 0x01, 0x008, 5),
    // XCONFIG_SECURED: mac_address, av_region, game_region, dvd_region, reset_key, system_flags, power_mode,
    // online_network_id, power_vcs_control
    (0x02, 0x01, 0x20, 6), (0x02, 0x02, 0x28, 4), (0x02, 0x03, 0x2C, 2), (0x02, 0x04, 0x34, 4), (0x02, 0x05, 0x38, 4),
    (0x02, 0x06, 0x3C, 4), (0x02, 0x07, 0x40, 2), (0x02, 0x08, 0x08, 4), (0x02, 0x09, 0x42, 2),
    // XCONFIG_USER: time zone (bias, std/dlt name, std/dlt date, std/dlt bias), default_profile, language,
    // video/audio/retail/devkit flags, country, parental_control_flags, smb_config, live_puid, live_credentials
    (0x03, 0x01, 0x008, 4), (0x03, 0x02, 0x00C, 4), (0x03, 0x03, 0x010, 4), (0x03, 0x04, 0x014, 4),
    (0x03, 0x05, 0x018, 4), (0x03, 0x06, 0x01C, 4), (0x03, 0x07, 0x020, 4), (0x03, 0x08, 0x024, 8),
    (0x03, 0x09, 0x02C, 4), (0x03, 0x0A, 0x030, 4), (0x03, 0x0B, 0x034, 4), (0x03, 0x0C, 0x038, 4),
    (0x03, 0x0D, 0x03C, 4), (0x03, 0x0E, 0x040, 1), (0x03, 0x0F, 0x041, 1), (0x03, 0x10, 0x044, 256),
    (0x03, 0x11, 0x144, 8), (0x03, 0x12, 0x14C, 16),
    // av_pack hdmi/component/vga screen sizes, parental control game/password/movie/ratings/hint/answer/override
    (0x03, 0x13, 0x15C, 4), (0x03, 0x14, 0x160, 4), (0x03, 0x15, 0x164, 4), (0x03, 0x16, 0x168, 4),
    (0x03, 0x17, 0x16C, 4), (0x03, 0x18, 0x170, 4), (0x03, 0x19, 0x174, 4), (0x03, 0x1A, 0x178, 4),
    (0x03, 0x1B, 0x17C, 1), (0x03, 0x1C, 0x17D, 32), (0x03, 0x1D, 0x19D, 32),
    // music playback mode/volume/flags, arcade flags, parental control version/tv/explicit/unrated video
    (0x03, 0x1E, 0x1BD, 4), (0x03, 0x1F, 0x1C1, 4), (0x03, 0x20, 0x1C5, 4), (0x03, 0x21, 0x1C9, 4),
    (0x03, 0x22, 0x1CD, 4), (0x03, 0x23, 0x1D1, 4), (0x03, 0x24, 0x1D5, 4), (0x03, 0x25, 0x1D9, 4),
    (0x03, 0x26, 0x1DD, 4), (0x03, 0x27, 0x1E1, 4), (0x03, 0x28, 0x1E5, 4),
    // black levels, player display mode, video timing ids, driver options, music ui flags, media source types
    (0x03, 0x29, 0x1E9, 4), (0x03, 0x2A, 0x1ED, 1), (0x03, 0x2B, 0x1EE, 4), (0x03, 0x2C, 0x1F2, 4),
    (0x03, 0x2D, 0x1F6, 4), (0x03, 0x2E, 0x1FA, 1), (0x03, 0x2F, 0x1FB, 1), (0x03, 0x30, 0x1FC, 1),
    // XCONFIG_CONSOLE: screen_saver, auto_shut_off, wireless_settings, camera_settings, play_timer_data,
    // media_disable_auto_launch, keyboard_layout
    (0x07, 0x01, 0x008, 2), (0x07, 0x02, 0x00A, 2), (0x07, 0x03, 0x00C, 256), (0x07, 0x04, 0x10C, 4),
    (0x07, 0x05, 0x12C, 20), (0x07, 0x06, 0x140, 2), (0x07, 0x07, 0x142, 2),
    // XCONFIG_IPTV: service_provider_name (60 UTF-16 characters)
    (0x09, 0x01, 0x008, 120),
    // XCONFIG_SYSTEM: alarm_time, previous_flash_version
    (0x0A, 0x01, 0x004, 8), (0x0A, 0x02, 0x00C, 4),
];

fn field(category: u16, setting: u16) -> Option<(usize, usize)> {
    FIELDS.iter().find(|f| f.0 == category && f.1 == setting).map(|f| (f.2, f.3 as usize))
}

/// XConfig: the category blocks of XConfigData.
#[derive(Debug, Clone)]
pub struct XConfig {
    blocks: Vec<Vec<u8>>,
}

impl Default for XConfig {
    /// XConfig::SetDefaults.
    fn default() -> Self {
        let mut c = XConfig { blocks: CATEGORY_SIZES.iter().map(|&n| vec![0; n]).collect() };
        let mut put = |category: u16, offset: usize, bytes: &[u8]| {
            c.blocks[category as usize][offset..offset + bytes.len()].copy_from_slice(bytes);
        };
        put(CATEGORY_SECURED, 0x28, &0x0040_0100u32.to_be_bytes()); // av_region = X_AV_REGION::NTSCM
        put(CATEGORY_USER, 0x02C, &USER_LANGUAGE.to_be_bytes());
        put(CATEGORY_USER, 0x040, &[USER_COUNTRY]);
        put(CATEGORY_USER, 0x034, &0x0001_0001u32.to_be_bytes()); // audio_flags = DolbyDigital | DolbyProLogic
        put(CATEGORY_USER, 0x15C, &0x0500_02D0u32.to_be_bytes()); // av_pack_hdmi_sz = XHDTVResolution[1], 1280x720
        put(CATEGORY_USER, 0x160, &0x0500_02D0u32.to_be_bytes()); // av_pack_component_sz = XHDTVResolution[1]
        put(CATEGORY_USER, 0x164, &0x02D0_0240u32.to_be_bytes()); // av_pack_vga_sz = XVGAResolution[3], 720x576
        put(CATEGORY_USER, 0x038, &0x0000_0040u32.to_be_bytes()); // retail_flags = DashboardInitialized
        put(CATEGORY_USER, 0x030, &0u32.to_be_bytes()); // video_flags = RatioNormal
        put(CATEGORY_USER, 0x041, &[0x03]); // parental_control_flags = XBLAllowed | XBLMembershipCreationAllowed
        put(CATEGORY_USER, 0x168, &0x0000_00FFu32.to_be_bytes()); // parental_control_game = NoGameRestrictions
        put(CATEGORY_USER, 0x1C1, &0.7f32.to_be_bytes()); // music_volume
        // kTimezones[0x19], "GMT+00 London": bias 0, GMT/BST, std {0A 05 00 02}, dlt {03 05 00 01}, dlt bias -60
        put(CATEGORY_USER, 0x008, &0i32.to_be_bytes());
        put(CATEGORY_USER, 0x00C, b"GMT\0");
        put(CATEGORY_USER, 0x010, b"BST\0");
        put(CATEGORY_USER, 0x014, &[0x0A, 0x05, 0x00, 0x02]);
        put(CATEGORY_USER, 0x018, &[0x03, 0x05, 0x00, 0x01]);
        put(CATEGORY_USER, 0x01C, &0i32.to_be_bytes());
        put(CATEGORY_USER, 0x020, &(-60i32).to_be_bytes());
        // iptv service_provider_name = u"Xenia TV" (big-endian UTF-16, truncated to the field)
        let name: Vec<u8> = "Xenia TV".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        put(CATEGORY_IPTV, 0x008, &name);
        c
    }
}

impl XConfig {
    /// XConfig::GetSettingSize: 0 for a setting that is not in the field table.
    pub fn setting_size(&self, category: u16, setting: u16) -> u16 {
        field(category, setting).map_or(0, |(_, size)| size as u16)
    }

    /// XConfig::ReadSetting: the setting's bytes as stored (big-endian); None when unknown.
    pub fn read(&self, category: u16, setting: u16) -> Option<&[u8]> {
        let (offset, size) = field(category, setting)?;
        Some(&self.blocks[category as usize][offset..offset + size])
    }

    /// XConfig::WriteSetting: the setting's size in bytes from `data`; unknown settings are ignored.
    pub fn write(&mut self, category: u16, setting: u16, data: &[u8]) {
        if let Some((offset, size)) = field(category, setting) {
            let n = size.min(data.len());
            self.blocks[category as usize][offset..offset + n].copy_from_slice(&data[..n]);
        }
    }

    /// XConfig::ReadSetting<uint32_t>: 0 unless the setting is 4 bytes.
    pub fn read_u32(&self, category: u16, setting: u16) -> u32 {
        match self.read(category, setting) {
            Some(b) if b.len() == 4 => u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            _ => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_english_and_the_united_states() {
        assert_eq!((super::language(), super::locale()), (1, 36));
        assert_eq!(super::LOCALE_FROM_COUNTRY[103], 36);
    }

    #[test]
    fn set_defaults_and_the_field_table_follow_xenia() {
        let mut c = XConfig::default();
        // static_asserts of xconfig.h
        assert_eq!((field(2, 1).unwrap().0, field(2, 2).unwrap().0, field(3, 9).unwrap().0), (32, 40, 44));
        assert_eq!((field(3, 0x0B).unwrap().0, field(3, 0x0C).unwrap().0, field(3, 0x0E).unwrap().0, field(3, 0x1F).unwrap().0), (52, 56, 64, 449));
        assert_eq!(c.read_u32(CATEGORY_USER, USER_RETAIL_FLAGS), 0x40, "DashboardInitialized");
        assert_eq!((c.read_u32(CATEGORY_SECURED, 2), c.read_u32(CATEGORY_USER, 9), c.read(CATEGORY_USER, 0x0E)), (0x0040_0100, 1, Some(&[103u8][..])));
        assert_eq!((c.read_u32(CATEGORY_USER, 0x13), c.read_u32(CATEGORY_USER, 0x15), c.read_u32(CATEGORY_USER, 0x16)), (0x0500_02D0, 0x02D0_0240, 0xFF));
        assert_eq!((c.read(CATEGORY_USER, 2), c.read_u32(CATEGORY_USER, 7) as i32), (Some(&b"GMT\0"[..]), -60));
        assert_eq!(f32::from_bits(c.read_u32(CATEGORY_USER, 0x1F)), 0.7);
        assert_eq!(&c.read(CATEGORY_IPTV, 1).unwrap()[..4], &[0, b'X', 0, b'e']);
        // unknown settings have no size and read nothing; writes keep to the field's size
        assert_eq!((c.setting_size(CATEGORY_CONSOLE, 0x09), c.read(CATEGORY_CONSOLE, 0x09)), (0, None));
        c.write(CATEGORY_USER, 0x0E, &[42, 0xEE]);
        assert_eq!((c.read(CATEGORY_USER, 0x0E), c.read(CATEGORY_USER, 0x0F)), (Some(&[42u8][..]), Some(&[3u8][..])));
    }
}
