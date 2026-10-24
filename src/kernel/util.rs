//! Guest-memory helpers shared by the HLE export handlers.

use crate::memory::MemoryBus;

/// Helper function to extract path string from guest memory (OBJECT_ATTRIBUTES or ANSI_STRING or C-string)
pub(super) fn read_vfs_path(p_obj_attr: u64, bus: &MemoryBus) -> Option<String> {
    if p_obj_attr == 0 {
        return None;
    }
    // Check if p_obj_attr has ObjectName pointer at offset 4
    let name_ptr = bus.read_u32(p_obj_attr + 4).unwrap_or(0) as u64;
    if name_ptr >= 0x1000 {
        // Try reading as ANSI_STRING: length at +0 (u16), max_length at +2 (u16), buffer at +4 (u32)
        let str_len = bus.read_u16(name_ptr).unwrap_or(0) as usize;
        let buf_ptr = bus.read_u32(name_ptr + 4).unwrap_or(0) as u64;
        if str_len > 0 && str_len < 1024 && buf_ptr >= 0x1000 {
            let mut bytes = Vec::new();
            for i in 0..str_len {
                if let Ok(b) = bus.read_u8(buf_ptr + i as u64) {
                    if b == 0 { break; }
                    bytes.push(b);
                }
            }
            if let Ok(s) = String::from_utf8(bytes.clone()) {
                if !s.is_empty() {
                    return Some(s);
                }
            } else {
                let s: String = bytes.iter().map(|&b| b as char).collect();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }

        // Also try reading name_ptr directly as null-terminated ASCII string
        let mut bytes = Vec::new();
        for i in 0..256 {
            if let Ok(b) = bus.read_u8(name_ptr + i as u64) {
                if b == 0 { break; }
                bytes.push(b);
            } else {
                break;
            }
        }
        if bytes.len() >= 3 && bytes.iter().all(|&b| (0x20..=0x7E).contains(&b) || b == b'\\' || b == b'/') {
            if let Ok(s) = String::from_utf8(bytes) {
                return Some(s);
            }
        }
    }

    // Fallback: Check if p_obj_attr itself is an ANSI_STRING
    let str_len = bus.read_u16(p_obj_attr).unwrap_or(0) as usize;
    let buf_ptr = bus.read_u32(p_obj_attr + 4).unwrap_or(0) as u64;
    if str_len > 0 && str_len < 1024 && buf_ptr >= 0x1000 {
        let mut bytes = Vec::new();
        for i in 0..str_len {
            if let Ok(b) = bus.read_u8(buf_ptr + i as u64) {
                if b == 0 { break; }
                bytes.push(b);
            }
        }
        if let Ok(s) = String::from_utf8(bytes) {
            if !s.is_empty() {
                return Some(s);
            }
        }
    }

    // Fallback: Check if p_obj_attr is directly a null-terminated ASCII string
    let mut bytes = Vec::new();
    for i in 0..256 {
        if let Ok(b) = bus.read_u8(p_obj_attr + i as u64) {
            if b == 0 { break; }
            bytes.push(b);
        } else {
            break;
        }
    }
    if bytes.len() >= 3 && bytes.iter().all(|&b| (0x20..=0x7E).contains(&b) || b == b'\\' || b == b'/') {
        if let Ok(s) = String::from_utf8(bytes) {
            return Some(s);
        }
    }

    None
}

/// Helper function to read null-terminated C string from guest memory
pub(super) fn read_c_string(ptr: u64, bus: &MemoryBus) -> Option<String> {
    if ptr == 0 { return None; }
    let mut bytes = Vec::new();
    for i in 0..512 {
        if let Ok(b) = bus.read_u8(ptr + i as u64) {
            if b == 0 { break; }
            bytes.push(b);
        } else {
            break;
        }
    }
    if bytes.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&bytes).to_string())
    }
}
