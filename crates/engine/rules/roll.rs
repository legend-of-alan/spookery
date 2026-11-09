//! Check definitions, one roll of a check, and the band its total lands in.

use std::collections::BTreeMap;

use crate::hash::blake2s_256;
use crate::rules::gate;
use crate::state::id::ActorId;
use crate::state::meter::{self, MeterDef};
use crate::state::modifier;
use crate::state::nested_map;
use crate::state::State;
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub dice_count: u32,
    pub dice_sides: u32,
    pub stat: String,
}

pub use spookery_protocol::Outcome;

/// `total` against `target`, with an optional lower band.
/// `total >= target` is success. `partial_at <= total < target` is partial.
/// Anything lower, and any binary roll, is a miss.
pub(crate) fn classify(total: i64, target: i64, partial_at: Option<i64>) -> Outcome {
    if total >= target {
        Outcome::Success
    } else if partial_at.is_some_and(|band| total >= band) {
        Outcome::Partial
    } else {
        Outcome::Miss
    }
}

/// One roll's dice, total, and band. The save does not store it, and the journal keeps only
/// the round's dice lines.
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
pub struct RollKey {
    pub world_id: String,
    pub round_index: u64,
    pub rule_id: String,
    /// The actor the rule runs for, or `None` for a change rule on a world change.
    pub subject_id: Option<ActorId>,
    pub index: u32,
}

impl RollKey {
    pub(crate) fn preimage(&self) -> String {
        let (world, round, rule, index) =
            (&self.world_id, self.round_index, &self.rule_id, self.index);
        match &self.subject_id {
            Some(subject) => format!("{world}|{round}|{rule}|{subject}|{index}"),
            None => format!("{world}|{round}|{rule}|{index}"),
        }
    }
}

/// The faces this key rolls from die `start` of its seed, without a stat or a band. A later
/// group in the same expression continues where the last one stopped.
pub fn faces(key: &RollKey, start: usize, count: u32, sides: u32) -> Result<Vec<u32>, Reason> {
    let seed = blake2s_256(key.preimage().as_bytes());
    let mut faces = Vec::with_capacity(count as usize);
    for n in 0..count {
        faces.push(face_at(&seed, start + n as usize, sides)?);
    }
    Ok(faces)
}

/// The only roll: `check`'s dice from this key's own seed, plus the actor's stat, against
/// `target`. `partial_at` `None` keeps the roll binary; an opposing total passes `None`.
pub(crate) fn roll_check(
    check: &Check,
    meters: &BTreeMap<String, MeterDef>,
    state: &State,
    actor_id: &ActorId,
    key: &RollKey,
    target: i64,
    partial_at: Option<i64>,
) -> Result<Roll, Reason> {
    let def = meters.get(&check.stat).ok_or(Reason::UnknownId)?;
    let live = state
        .actors
        .meters
        .get(&(actor_id.clone(), check.stat.clone()))
        .ok_or(Reason::UnknownId)?;
    let held = nested_map::entries_of(&state.actors.modifiers, actor_id).map(|(_, held)| held);
    let stat = meter::current(def, live, modifier::sum(held, &check.stat)?)?;
    let faces = faces(key, 0, check.dice_count, check.dice_sides)?;
    let total = faces.iter().map(|face| i64::from(*face)).sum::<i64>() + stat;
    Ok(Roll {
        faces,
        stat,
        total,
        target,
        outcome: classify(total, target, partial_at),
    })
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
    use std::collections::BTreeMap;

    use super::{classify, face_at, roll_check, Check, Outcome, RollKey};
    use crate::state::id::ActorId;
    use crate::state::meter::{Meter, MeterDef};
    use crate::state::{Act, ActorKind, State};

    fn grit_def() -> MeterDef {
        MeterDef {
            min: 0,
            base_cap: 100,
            max: 110,
            start: 1,
            hidden: false,
            carrier: None,
        }
    }

    /// A save holding one NPC, `mira`, with grit 1.
    fn npc() -> State {
        let mut state = State {
            schema: "save".to_string(),
            world_id: "docks".to_string(),
            pack_rev: String::new(),
            rules_rev: 0,
            state_rev: 0,
            round_index: 0,
            world_time: 0,
            last_round_at: String::new(),
            actors: Default::default(),
            places: Default::default(),
            stories: Default::default(),
            world: Default::default(),
            notes: Vec::new(),
        };
        let (actors, mira) = (&mut state.actors, ActorId::from("mira"));
        actors.kind.insert(mira.clone(), ActorKind::Npc);
        actors.place.insert(mira.clone(), "mira_rooms".to_string());
        actors.acts.insert(mira.clone(), Act::default());
        let grit = Meter { base: 1, offset: 0 };
        actors.meters.insert((mira, "grit".to_string()), grit);
        state
    }

    fn check_of(count: u32) -> Check {
        Check {
            dice_count: count,
            dice_sides: 6,
            stat: "grit".to_string(),
        }
    }

    fn key(index: u32) -> RollKey {
        RollKey {
            world_id: "docks".to_string(),
            round_index: 1,
            rule_id: "grit".to_string(),
            subject_id: Some("mira".into()),
            index,
        }
    }

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
    fn the_same_key_rolls_the_same_faces() {
        let (state, mira) = (npc(), ActorId::from("mira"));
        let defs = BTreeMap::from([("grit".to_string(), grit_def())]);
        let check = check_of(2);
        let once = roll_check(&check, &defs, &state, &mira, &key(0), 9, None).unwrap();
        let twice = roll_check(&check, &defs, &state, &mira, &key(0), 9, None).unwrap();
        assert_eq!(once, twice);
        assert_eq!(once.faces.len(), 2);
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
        for (total, target, partial_at, expected) in cases {
            assert_eq!(
                classify(total, target, partial_at),
                expected,
                "{total} vs {target} band {partial_at:?}"
            );
        }
    }
}
