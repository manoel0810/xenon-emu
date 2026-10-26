// Derived from Xenia (BSD-3-Clause), Copyright (c) 2013 Ben Vanik; file headers: Copyright 2026 Xenia Canary.
// See docs/THIRD_PARTY.md. Xenia Canary commit 451f915566d57436555c6eb4ee77bb4717eb01de:
//   src/xenia/kernel/xconfig.cc          XConfig::SetDefaults (user language and country)
//   src/xenia/kernel/xam/xam_locale.cc   xeXamGetLocaleFromCountry, xeXamGetLocaleEx, xeGetLanguage
//   src/xenia/xbox.h                     XLanguage, XOnlineCountry
//
//! The user settings of the console configuration (XConfig) that XAM reads, at Xenia's defaults. The rest of
//! XConfig (ExGetXConfigSetting) is not emulated yet.

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

#[cfg(test)]
mod tests {
    #[test]
    fn defaults_are_english_and_the_united_states() {
        assert_eq!((super::language(), super::locale()), (1, 36));
        assert_eq!(super::LOCALE_FROM_COUNTRY[103], 36);
    }
}
