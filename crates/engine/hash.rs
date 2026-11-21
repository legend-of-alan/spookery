//! Blake2s, as raw bytes, as hex, or as hex tagged `blake2s:`.

use blake2::{Blake2s256, Digest};

pub(crate) fn blake2s_256(bytes: &[u8]) -> [u8; 32] {
    Blake2s256::digest(bytes).into()
}

pub(crate) fn blake2s_hex(bytes: &[u8]) -> String {
    blake2s_256(bytes).map(|b| format!("{b:02x}")).concat()
}

#[cfg_attr(not(feature = "testing"), expect(unreachable_pub))]
pub fn blake2s_tagged(bytes: &[u8]) -> String {
    format!("blake2s:{}", blake2s_hex(bytes))
}
