use std::collections::BTreeMap;

use crate::check::{classify, CheckDef, Outcome};
use crate::error::Reason;
use crate::hash::blake2s_256;
use crate::intent::ViewerId;
use crate::meter::{self, MeterDef};
use crate::model::ActorLive;

/// One roll's dice, total, and band. Not stored in `Now`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollResult {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub outcome: Outcome,
}

/// A roll the round resolved. Not stored in `Now` or the tick JSONL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RollReport {
    Rolled { actor_id: String, out: RollResult },
    Refused { actor_id: String, code: Reason },
}

impl RollReport {
    pub fn actor_id(&self) -> &str {
        match self {
            RollReport::Rolled { actor_id, .. } | RollReport::Refused { actor_id, .. } => actor_id,
        }
    }
}

/// Identifies one roll. A later `resist:` roll uses the same shape: subject plus index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollKey {
    pub world_id: String,
    pub round_index: u64,
    pub rule_id: String,
    pub subject_id: String,
    pub roll_index: u32,
}

impl RollKey {
    pub(crate) fn preimage(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}",
            self.world_id, self.round_index, self.rule_id, self.subject_id, self.roll_index
        )
    }
}

pub struct RollInput<'a> {
    pub check: &'a CheckDef,
    pub defs: &'a BTreeMap<String, MeterDef>,
    pub actor_id: &'a str,
    pub actor: &'a ActorLive,
    pub viewer: &'a ViewerId,
    pub target: i64,
    /// `None` keeps the roll binary. An opposing total passes `None`.
    pub partial_at: Option<i64>,
}

/// The faces this key would roll, without a stat or a band.
pub fn faces(key: &RollKey, count: u32, sides: u32) -> Result<Vec<u32>, Reason> {
    let seed = blake2s_256(key.preimage().as_bytes());
    let mut faces = Vec::with_capacity(count as usize);
    for n in 0..count {
        faces.push(face_at(&seed, n as usize, sides)?);
    }
    Ok(faces)
}

/// The only roll. The viewer must be the actor. Each die reads this key's own seed.
pub fn roll(input: RollInput<'_>, key: &RollKey) -> Result<RollResult, Reason> {
    if input.viewer.actor_id() != input.actor_id {
        return Err(Reason::UnknownKey);
    }
    let def = input
        .defs
        .get(&input.check.stat)
        .ok_or(Reason::UnknownKey)?;
    let live = input
        .actor
        .meters()
        .get(&input.check.stat)
        .ok_or(Reason::UnknownKey)?;
    let stat = meter::current(def, live)?;
    let seed = blake2s_256(key.preimage().as_bytes());
    let mut faces = Vec::with_capacity(input.check.dice_count as usize);
    for n in 0..input.check.dice_count {
        faces.push(face_at(&seed, n as usize, input.check.dice_sides)?);
    }
    let total = faces.iter().map(|face| i64::from(*face)).sum::<i64>() + stat;
    Ok(RollResult {
        faces,
        stat,
        total,
        target: input.target,
        outcome: classify(total, input.target, input.partial_at),
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

    use super::{face_at, roll, RollInput, RollKey};
    use crate::check::{CheckDef, Outcome};
    use crate::error::Reason;
    use crate::intent::ViewerId;
    use crate::meter::{MeterDef, MeterLive};
    use crate::model::{ActCounters, ActorLive};

    fn grit_def() -> MeterDef {
        MeterDef {
            min: 0,
            base_cap: 100,
            max: 110,
            start: 1,
        }
    }

    fn npc() -> ActorLive {
        ActorLive::Npc {
            place: "mira_rooms".to_string(),
            flags: Vec::new(),

            meters: BTreeMap::from([("grit".to_string(), MeterLive { base: 1, offset: 0 })]),
            inventory: BTreeMap::new(),
            wielded: None,
            known_facts: Vec::new(),
            act: ActCounters::fresh(),
        }
    }

    fn check_of(count: u32) -> CheckDef {
        CheckDef {
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
            subject_id: "mira".to_string(),
            roll_index: index,
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
        let actor = npc();
        let defs = BTreeMap::from([("grit".to_string(), grit_def())]);
        let check = check_of(2);
        let once = roll(
            RollInput {
                check: &check,
                defs: &defs,
                actor_id: "mira",
                actor: &actor,
                viewer: &ViewerId::Npc("mira".to_string()),
                target: 9,
                partial_at: None,
            },
            &key(0),
        )
        .unwrap();
        let twice = roll(
            RollInput {
                check: &check,
                defs: &defs,
                actor_id: "mira",
                actor: &actor,
                viewer: &ViewerId::Player("mira".to_string()),
                target: 9,
                partial_at: None,
            },
            &key(0),
        )
        .unwrap();
        assert_eq!(once, twice);
        assert_eq!(once.faces.len(), 2);
    }

    #[test]
    fn a_different_viewer_is_refused() {
        let actor = npc();
        let defs = BTreeMap::from([("grit".to_string(), grit_def())]);
        let check = check_of(2);
        let refused = roll(
            RollInput {
                check: &check,
                defs: &defs,
                actor_id: "mira",
                actor: &actor,
                viewer: &ViewerId::Npc("bram".to_string()),
                target: 9,
                partial_at: None,
            },
            &key(0),
        );
        assert_eq!(refused, Err(Reason::UnknownKey));
    }

    #[test]
    fn ten_dice_in_one_roll_are_not_refused() {
        let actor = npc();
        let defs = BTreeMap::from([("grit".to_string(), grit_def())]);
        let check = check_of(10);
        let out = roll(
            RollInput {
                check: &check,
                defs: &defs,
                actor_id: "mira",
                actor: &actor,
                viewer: &ViewerId::Npc("mira".to_string()),
                target: 0,
                partial_at: None,
            },
            &key(0),
        )
        .unwrap();
        assert_eq!(out.faces.len(), 10);
        assert_eq!(out.outcome, Outcome::Success);
    }
}
