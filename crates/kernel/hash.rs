//! Blake2s, as raw bytes or as hex.

use blake2::{Blake2s256, Digest};

pub fn blake2s_256(bytes: &[u8]) -> [u8; 32] {
    let digest = Blake2s256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn blake2s_hex(bytes: &[u8]) -> String {
    hex_encode(&blake2s_256(bytes))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
