//! Roll keys, the faces a key rolls, and the band a check's total lands in.

use crate::ecs::id::ActorId;
use crate::hash::blake2s_256;
use crate::rules::gate;
use crate::Reason;

pub use spookery_protocol::Outcome;

/// `total` against `threshold`, with an optional lower band.
/// `total >= threshold` is success. `partial_at <= total < threshold` is partial.
/// Anything lower, and any binary roll, is a miss.
pub(crate) fn classify(total: i64, threshold: i64, partial_at: Option<i64>) -> Outcome {
    if total >= threshold {
        Outcome::Success
    } else if partial_at.is_some_and(|band| total >= band) {
        Outcome::Partial
    } else {
        Outcome::Miss
    }
}

/// One check's total, the bar it met or missed, and its band. The save does not store it, and
/// the journal keeps only the round's dice lines.
pub use spookery_protocol::Roll;

/// What the round did with a move: the roll, or a refusal and its reason. Returned from the
/// engine's round to the host table, which turns it into check frames. Neither the save nor
/// the journal stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoveReport {
    Rolled {
        actor_id: ActorId,
        roll: Roll,
    },
    Refused {
        actor_id: ActorId,
        refusal: gate::Refusal,
    },
}

/// Identifies one roll. A later `resist:` roll uses the same shape: subject plus index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    /// The save's dice seed.
    pub seed: String,
    pub round_index: u64,
    pub rule_id: String,
    /// The actor the rule runs for, or `None` for a change rule on a world change.
    pub subject_id: Option<ActorId>,
    pub index: u32,
}

impl Key {
    pub(crate) fn preimage(&self) -> String {
        let (seed, round, rule, index) = (&self.seed, self.round_index, &self.rule_id, self.index);
        match &self.subject_id {
            Some(subject) => format!("{seed}|{round}|{rule}|{subject}|{index}"),
            None => format!("{seed}|{round}|{rule}|{index}"),
        }
    }
}

/// The faces this key rolls from die `start` of its seed, without a stat or a band. A later
/// group in the same expression continues where the last one stopped.
pub fn faces(key: &Key, start: usize, count: u32, sides: u32) -> Result<Vec<u32>, Reason> {
    let seed = blake2s_256(key.preimage().as_bytes());
    let mut faces = Vec::with_capacity(count as usize);
    for n in 0..count {
        faces.push(face_at(&seed, start + n as usize, sides)?);
    }
    Ok(faces)
}

/// Die `n` reads 4 bytes from the roll seed. Past 8 dice, block `k` is `blake2s(seed || k)`.
fn face_at(seed: &[u8; 32], n: usize, sides: u32) -> Result<u32, Reason> {
    if sides == 0 {
        return Err(Reason::Schema);
    }
    let bytes = block_bytes(seed, n / 8);
    let start = (n % 8) * 4;
    let mut word = [0u8; 4];
    word.copy_from_slice(&bytes[start..start + 4]);
    Ok((u32::from_le_bytes(word) % sides) + 1)
}

fn block_bytes(seed: &[u8; 32], block: usize) -> [u8; 32] {
    if block == 0 {
        return *seed;
    }
    let mut preimage = Vec::with_capacity(36);
    preimage.extend_from_slice(seed);
    preimage.extend_from_slice(&(block as u32).to_le_bytes());
    blake2s_256(&preimage)
}

#[cfg(test)]
mod tests {
    use super::{classify, face_at, Outcome};

    fn seed_of(words: &[u32]) -> [u8; 32] {
        let mut seed = [0u8; 32];
        for (index, word) in words.iter().enumerate() {
            let start = index * 4;
            seed[start..start + 4].copy_from_slice(&word.to_le_bytes());
        }
        seed
    }

    #[test]
    fn a_word_maps_to_a_face() {
        let seed = seed_of(&[5, 0]);
        assert_eq!(face_at(&seed, 0, 6).unwrap(), 6);
        assert_eq!(face_at(&seed, 1, 6).unwrap(), 1);
    }

    #[test]
    fn ten_dice_read_past_the_first_block() {
        let seed = seed_of(&[0]);
        let face = face_at(&seed, 9, 6).unwrap();
        assert!((1..=6).contains(&face));
    }

    #[test]
    fn classification_meets_every_threshold() {
        let success = Outcome::Success;
        let partial = Outcome::Partial;
        let miss = Outcome::Miss;
        let cases = [
            (10, 10, None, success),
            (9, 10, None, miss),
            (11, 10, None, success),
            (10, 10, Some(7), success),
            (9, 10, Some(7), partial),
            (7, 10, Some(7), partial),
            (6, 10, Some(7), miss),
            (11, 10, Some(7), success),
            (0, 0, None, success),
            (0, 0, Some(-1), success),
            (-1, 0, Some(-1), partial),
            (-2, 0, Some(-1), miss),
        ];
        for (total, threshold, partial_at, expected) in cases {
            assert_eq!(
                classify(total, threshold, partial_at),
                expected,
                "{total} vs {threshold} band {partial_at:?}"
            );
        }
    }
}
