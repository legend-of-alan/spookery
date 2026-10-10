use std::collections::BTreeMap;

use serde::Deserialize;

use crate::error::Reason;
use crate::intent::ViewerId;
use crate::meter::{self, MeterDef};
use crate::model::ActorLive;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckDef {
    pub dice_count: u32,
    pub dice_sides: u32,
    pub stat: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOut {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub success: bool,
}

/// A check the tick resolved. Not stored in `Now` or the tick JSONL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckNotice {
    Rolled { actor_id: String, out: CheckOut },
    Refused { actor_id: String, code: Reason },
}

impl CheckNotice {
    pub fn actor_id(&self) -> &str {
        match self {
            CheckNotice::Rolled { actor_id, .. } | CheckNotice::Refused { actor_id, .. } => {
                actor_id
            }
        }
    }
}

pub fn load_checks(text: &str) -> Result<BTreeMap<String, CheckDef>, Reason> {
    crate::format::require_tag(text, crate::format::CHECKS)?;
    let file: ChecksFile = serde_saphyr::from_str(text).map_err(|_| Reason::Schema)?;
    if file.schema != crate::format::CHECKS {
        return Err(Reason::Schema);
    }
    let mut checks = BTreeMap::new();
    for entry in file.checks {
        if entry.id.is_empty()
            || entry.dice.count == 0
            || entry.dice.sides == 0
            || checks
                .insert(
                    entry.id,
                    CheckDef {
                        dice_count: entry.dice.count,
                        dice_sides: entry.dice.sides,
                        stat: entry.stat,
                    },
                )
                .is_some()
        {
            return Err(Reason::Schema);
        }
    }
    Ok(checks)
}

pub fn die_face(rng: &[u8], n: usize, sides: u32) -> Result<u32, Reason> {
    if sides == 0 {
        return Err(Reason::Schema);
    }
    let start = n.checked_mul(4).ok_or(Reason::NotInSpec)?;
    let end = start.checked_add(4).ok_or(Reason::NotInSpec)?;
    if end > rng.len() || end > 32 {
        return Err(Reason::NotInSpec);
    }
    let word = u32::from_le_bytes(rng[start..end].try_into().map_err(|_| Reason::NotInSpec)?);
    Ok((word % sides) + 1)
}

pub struct CheckRoll<'a> {
    pub check: &'a CheckDef,
    pub defs: &'a BTreeMap<String, MeterDef>,
    pub actor_id: &'a str,
    pub actor: &'a ActorLive,
    pub viewer: &'a ViewerId,
    pub target: i64,
    pub rng: &'a [u8],
    pub die_cursor: &'a mut usize,
}

/// The viewer must be the actor. A PC or an NPC may roll for themselves.
/// A different viewer is `UNKNOWN_KEY` and does not advance `die_cursor`.
pub fn resolve_check(roll: CheckRoll<'_>) -> Result<CheckOut, Reason> {
    if roll.viewer.actor_id() != roll.actor_id {
        return Err(Reason::UnknownKey);
    }
    let def = roll.defs.get(&roll.check.stat).ok_or(Reason::UnknownKey)?;
    let live = roll
        .actor
        .meters()
        .get(&roll.check.stat)
        .ok_or(Reason::UnknownKey)?;
    let stat = meter::current(def, live)?;
    let mut faces = Vec::with_capacity(roll.check.dice_count as usize);
    for _ in 0..roll.check.dice_count {
        let face = die_face(roll.rng, *roll.die_cursor, roll.check.dice_sides)?;
        *roll.die_cursor += 1;
        faces.push(face);
    }
    let total = faces.iter().map(|face| i64::from(*face)).sum::<i64>() + stat;
    Ok(CheckOut {
        faces,
        stat,
        total,
        target: roll.target,
        success: total >= roll.target,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksFile {
    schema: String,
    checks: Vec<CheckEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckEntry {
    id: String,
    dice: Dice,
    stat: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dice {
    count: u32,
    sides: u32,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{load_checks, resolve_check, CheckDef, CheckOut, CheckRoll};
    use crate::error::Reason;
    use crate::intent::ViewerId;
    use crate::meter::{MeterDef, MeterLive};
    use crate::model::{ActorLive, TickCounters};

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
            busy: false,
            meters: BTreeMap::from([("grit".to_string(), MeterLive { base: 1, offset: 0 })]),
            inventory: Vec::new(),
            known_facts: Vec::new(),
            tick: TickCounters {
                last_act_tick: 0,
                idle_acts_today: 0,
                live_acts_this_hour: 0,
            },
        }
    }

    fn check() -> CheckDef {
        CheckDef {
            dice_count: 2,
            dice_sides: 6,
            stat: "grit".to_string(),
        }
    }

    /// Face is `(word % 6) + 1`. Word 5 is a 6. Word 0 is a 1.
    fn rng_of(words: &[u32]) -> [u8; 32] {
        let mut rng = [0u8; 32];
        for (index, word) in words.iter().enumerate() {
            let start = index * 4;
            rng[start..start + 4].copy_from_slice(&word.to_le_bytes());
        }
        rng
    }

    fn roll(
        viewer: &ViewerId,
        actor_id: &str,
        target: i64,
        rng: &[u8],
        die_cursor: &mut usize,
    ) -> Result<CheckOut, Reason> {
        let actor = npc();
        let defs = BTreeMap::from([("grit".to_string(), grit_def())]);
        let check = check();
        resolve_check(CheckRoll {
            check: &check,
            defs: &defs,
            actor_id,
            actor: &actor,
            viewer,
            target,
            rng,
            die_cursor,
        })
    }

    #[test]
    fn an_npc_succeeds_its_own_check() {
        let rng = rng_of(&[5, 5]);
        let mut cursor = 0usize;
        let out = roll(
            &ViewerId::Npc("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        )
        .unwrap();
        assert_eq!(out.faces, vec![6, 6]);
        assert_eq!(out.stat, 1);
        assert_eq!(out.total, 13);
        assert_eq!(out.target, 9);
        assert!(out.success);
        assert_eq!(cursor, 2);
    }

    #[test]
    fn an_npc_fails_its_own_check() {
        let rng = rng_of(&[0, 0]);
        let mut cursor = 0usize;
        let out = roll(
            &ViewerId::Npc("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        )
        .unwrap();
        assert_eq!(out.faces, vec![1, 1]);
        assert_eq!(out.total, 3);
        assert!(!out.success);
        assert_eq!(cursor, 2);
    }

    #[test]
    fn a_player_rolling_for_another_actor_is_refused() {
        let rng = rng_of(&[5, 5]);
        let mut cursor = 0usize;
        let refused = roll(
            &ViewerId::Player("pc_rowan".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        );
        assert_eq!(refused, Err(Reason::UnknownKey));
        assert_eq!(cursor, 0);
    }

    #[test]
    fn an_npc_rolling_for_another_actor_is_refused() {
        let rng = rng_of(&[5, 5]);
        let mut cursor = 0usize;
        let refused = roll(
            &ViewerId::Npc("bram".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        );
        assert_eq!(refused, Err(Reason::UnknownKey));
        assert_eq!(cursor, 0);
    }

    #[test]
    fn the_die_cursor_advances_only_on_a_real_roll() {
        // Words 5, 5, 5, 0. A refusal that moved the cursor would read the last two as 1, 1.
        let rng = rng_of(&[5, 5, 5, 0]);
        let mut cursor = 0usize;
        let first = roll(
            &ViewerId::Npc("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        )
        .unwrap();
        assert_eq!(first.faces, vec![6, 6]);
        assert_eq!(cursor, 2);
        let refused = roll(
            &ViewerId::Player("pc_rowan".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        );
        assert_eq!(refused, Err(Reason::UnknownKey));
        assert_eq!(cursor, 2);
        let second = roll(
            &ViewerId::Npc("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut cursor,
        )
        .unwrap();
        assert_eq!(second.faces, vec![6, 1]);
        assert_eq!(cursor, 4);
    }

    #[test]
    fn the_same_seed_resolves_the_same_check() {
        let rng = rng_of(&[5, 0]);
        let mut first_cursor = 0usize;
        let mut second_cursor = 0usize;
        let mut player_cursor = 0usize;
        let first = roll(
            &ViewerId::Npc("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut first_cursor,
        )
        .unwrap();
        let second = roll(
            &ViewerId::Npc("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut second_cursor,
        )
        .unwrap();
        let player = roll(
            &ViewerId::Player("mira".to_string()),
            "mira",
            9,
            &rng,
            &mut player_cursor,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(first, player);
        assert_eq!(first.faces, vec![6, 1]);
        assert_eq!(first_cursor, 2);
        assert_eq!(second_cursor, 2);
        assert_eq!(player_cursor, 2);
    }

    fn text(tag: &str) -> String {
        format!("schema: {tag}\nchecks: []\n")
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        assert!(load_checks(&text("checks")).is_ok());
        assert_eq!(
            load_checks(&text(concat!("checks/", "v0"))).unwrap_err(),
            Reason::Schema
        );
        assert_eq!(load_checks(&text("bogus")).unwrap_err(), Reason::Schema);
    }
}
