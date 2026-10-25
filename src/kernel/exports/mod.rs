//! Canonical Xbox 360 Kernel & XAM Export Names (from Official Xenia Canary tables).

mod xam_names;
mod xboxkrnl_names;

pub use xam_names::get_xam_name;
pub use xboxkrnl_names::get_xboxkrnl_name;

#[cfg(test)]
mod tests {
    use super::*;

    const XBOXKRNL_SOURCES: &[(&str, &str)] = &[
        ("ex.rs", include_str!("../xboxkrnl/ex.rs")),
        ("ke.rs", include_str!("../xboxkrnl/ke.rs")),
        ("misc.rs", include_str!("../xboxkrnl/misc.rs")),
        ("mm.rs", include_str!("../xboxkrnl/mm.rs")),
        ("nt.rs", include_str!("../xboxkrnl/nt.rs")),
        ("rtl.rs", include_str!("../xboxkrnl/rtl.rs")),
        ("vd.rs", include_str!("../xboxkrnl/vd.rs")),
        ("xe_crypt.rs", include_str!("../xboxkrnl/xe_crypt.rs")),
    ];
    const XAM_SOURCES: &[(&str, &str)] = &[
        ("app.rs", include_str!("../xam/app.rs")),
        ("content.rs", include_str!("../xam/content.rs")),
        ("input.rs", include_str!("../xam/input.rs")),
        ("misc.rs", include_str!("../xam/misc.rs")),
        ("net.rs", include_str!("../xam/net.rs")),
        ("notify.rs", include_str!("../xam/notify.rs")),
        ("ui.rs", include_str!("../xam/ui.rs")),
        ("user.rs", include_str!("../xam/user.rs")),
    ];

    /// Every `0xNNNN: Name` in a comment of the HLE sources, as (line, ordinal, name).
    fn labelled_ordinals(source: &str) -> Vec<(usize, u16, String)> {
        let mut found = Vec::new();
        for (n, line) in source.lines().enumerate() {
            let Some(comment) = line.trim_start().strip_prefix("//") else { continue };
            let mut rest = comment;
            while let Some(pos) = rest.find("0x") {
                let after = &rest[pos + 2..];
                let hex: String = after.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
                let tail = &after[hex.len()..];
                if hex.len() == 4 {
                    if let Some(named) = tail.strip_prefix(':') {
                        let name: String = named.trim_start().chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                        if !name.is_empty() {
                            found.push((n + 1, u16::from_str_radix(&hex, 16).unwrap(), name));
                        }
                    }
                }
                rest = tail;
            }
        }
        found
    }

    #[test]
    fn hle_comments_name_the_real_exports() {
        let mut wrong = Vec::new();
        for (sources, lookup, lib) in [
            (XBOXKRNL_SOURCES, get_xboxkrnl_name as fn(u16) -> Option<&'static str>, "xboxkrnl"),
            (XAM_SOURCES, get_xam_name as fn(u16) -> Option<&'static str>, "xam"),
        ] {
            for (file, source) in sources {
                for (line, ordinal, name) in labelled_ordinals(source) {
                    if lookup(ordinal) != Some(name.as_str()) {
                        wrong.push(format!("{}/{}:{} 0x{:04X} says {} but is {:?}", lib, file, line, ordinal, name, lookup(ordinal)));
                    }
                }
            }
        }
        assert!(wrong.is_empty(), "mislabelled ordinals:\n{}", wrong.join("\n"));
    }

    #[test]
    fn label_parser_finds_names() {
        let found = labelled_ordinals("            // 0x0140: RtlTimeToTimeFields(a) | 0x0141: RtlTryEnterCriticalSection\n x // 0x0001: NotAComment\n");
        assert_eq!(found, vec![(1, 0x0140, "RtlTimeToTimeFields".to_string()), (1, 0x0141, "RtlTryEnterCriticalSection".to_string())]);
    }
}
