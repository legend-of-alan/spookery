use spookery_kernel::blake2s_256;

/// The tick line's `seed` field. A roll uses `RollKey`, not this string.
pub fn tick_seed(shard_id: &str, tick_index: u64) -> String {
    format!("{shard_id}|{tick_index}")
}

pub fn shuffle_key(shard_id: &str, tick_index: u64, actor_id: &str) -> [u8; 32] {
    blake2s_256(format!("{shard_id}|{tick_index}|{actor_id}").as_bytes())
}
