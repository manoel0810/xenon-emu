pub mod rc4;

use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Factory 1BL retail key embedded in all retail Xbox 360 CPU ROMs
pub const ONE_BL_RETAIL_KEY: [u8; 16] = [
    0xDD, 0x88, 0xAD, 0x0C, 0x9E, 0xD6, 0x69, 0xE7,
    0xB5, 0x67, 0x94, 0xFB, 0x68, 0x56, 0x3E, 0xFA,
];

pub fn derive_cba_key(salt: &[u8]) -> Result<[u8; 16], String> {
    let mut mac = HmacSha1::new_from_slice(&ONE_BL_RETAIL_KEY)
        .map_err(|e| format!("HMAC init failed: {:?}", e))?;
    mac.update(salt);
    let digest = mac.finalize().into_bytes();
    let mut key = [0u8; 16];
    key.copy_from_slice(&digest[..16]);
    Ok(key)
}

pub fn decrypt_bootloader_stage(data: &[u8], key: &[u8]) -> Vec<u8> {
    let mut result = data.to_vec();
    if result.len() > 0x20 {
        let mut cipher = rc4::Rc4::new(key);
        cipher.crypt(&mut result[0x20..]);
    }
    result
}
