use byteorder::{BigEndian, ByteOrder};
use std::fs;
use std::path::Path;

/// Parser for authentic Xbox 360 Localized String Tables (.xus)
#[derive(Debug, Clone)]
pub struct XusTable {
    pub strings: Vec<String>,
}

impl XusTable {
    /// Parses an in-memory .xus buffer containing UTF-16 BE strings
    pub fn parse(bytes: &[u8]) -> Self {
        let u16_chars: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| BigEndian::read_u16(c))
            .collect();
        let text = String::from_utf16_lossy(&u16_chars);

        let mut strings = Vec::new();
        for s in text.split('\0') {
            let trimmed = s.trim();
            // Filter out control symbols or empty fragments
            if trimmed.len() >= 2 && trimmed.chars().any(|c| c.is_alphabetic()) {
                strings.push(trimmed.to_string());
            }
        }

        Self { strings }
    }

    /// Loads and parses an .xus string table file from disk
    pub fn load_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let data = fs::read(path.as_ref())
            .map_err(|e| format!("Failed to read .xus file '{}': {:?}", path.as_ref().display(), e))?;
        Ok(Self::parse(&data))
    }

    /// Finds a string containing a specific substring (case-insensitive)
    pub fn find(&self, query: &str) -> Option<&String> {
        self.strings
            .iter()
            .find(|s| s.to_lowercase().contains(&query.to_lowercase()))
    }
}
