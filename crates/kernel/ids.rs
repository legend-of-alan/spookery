use crate::hash::blake2s_hex;

/// Blake2s of `shard|tick|actor|row`. The P37 preimage stays.
pub fn kernel_intent_id(shard_id: &str, tick_index: u64, actor_id: &str, row: &str) -> String {
    blake2s_hex(format!("{shard_id}|{tick_index}|{actor_id}|{row}").as_bytes())
}

/// Blake2s of `shard|tick_index_plus_one|actor|canonical_line`.
pub fn player_intent_id(
    shard_id: &str,
    tick_index_plus_one: u64,
    actor_id: &str,
    line: &str,
) -> String {
    blake2s_hex(format!("{shard_id}|{tick_index_plus_one}|{actor_id}|{line}").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_intent_id_matches_pin_p37() {
        assert_eq!(
            kernel_intent_id("cap", 1, "n2", "work"),
            "daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819"
        );
    }
}
