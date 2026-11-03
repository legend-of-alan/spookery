//! Blake2s, as raw bytes, as hex, or as hex tagged `blake2s:`.

use blake2::{Blake2s256, Digest};

pub(crate) fn blake2s_256(bytes: &[u8]) -> [u8; 32] {
    let digest = Blake2s256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn blake2s_hex(bytes: &[u8]) -> String {
    format!("{:x}", Blake2s256::digest(bytes))
}

pub fn blake2s_tagged(bytes: &[u8]) -> String {
    format!("blake2s:{}", blake2s_hex(bytes))
}
