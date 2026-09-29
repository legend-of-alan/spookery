use spookery_kernel::{blake2s_256, die_face as kernel_die_face, Reason};

pub fn seed_preimage(shard_id: &str, tick_index: u64) -> String {
    format!("{shard_id}|{tick_index}")
}

pub fn shuffle_key(shard_id: &str, tick_index: u64, actor_id: &str) -> [u8; 32] {
    blake2s_256(format!("{shard_id}|{tick_index}|{actor_id}").as_bytes())
}

pub fn die_face(rng: &[u8], n: usize, sides: u32) -> Result<u32, Reason> {
    kernel_die_face(rng, n, sides)
}
