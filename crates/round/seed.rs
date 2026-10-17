//! The round line seed, and the key that shuffles intents.

use spookery_kernel::api::blake2s_256;

/// The round line's `seed` field. A roll uses `RollKey`, not this string.
pub fn round_seed(world_id: &str, tick_index: u64) -> String {
    format!("{world_id}|{tick_index}")
}

pub fn shuffle_key(world_id: &str, tick_index: u64, actor_id: &str) -> [u8; 32] {
    blake2s_256(format!("{world_id}|{tick_index}|{actor_id}").as_bytes())
}
