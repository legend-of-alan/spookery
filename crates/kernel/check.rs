use std::collections::BTreeMap;

use serde::Deserialize;

use crate::diagnostic::{locate_row_field, parse_file, schema, tag_diagnostic, Diagnostic};
use crate::error::Reason;
use crate::intent::ViewerId;

const CHECKS_FILE: &str = "rules/checks.yaml";
use crate::meter::{self, MeterDef};
use crate::model::ActorLive;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckDef {
    pub dice_count: u32,
    pub dice_sides: u32,
    pub stat: String,
}

/// One roll's band. A row without `partial_at` is `Success` or `Miss`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Partial,
    Miss,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Miss => "miss",
        }
    }
}

/// `total` against `target`, with an optional lower band.
/// `total >= target` is success. `partial_at <= total < target` is partial.
/// Anything lower, and any binary roll, is a miss.
pub fn classify(total: i64, target: i64, partial_at: Option<i64>) -> Outcome {
    if total >= target {
        Outcome::Success
    } else if partial_at.is_some_and(|band| total >= band) {
        Outcome::Partial
    } else {
        Outcome::Miss
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOut {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub outcome: Outcome,
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

pub fn load_checks(text: &str) -> Result<BTreeMap<String, CheckDef>, Vec<Diagnostic>> {
    if let Some(diag) = tag_diagnostic(text, crate::format::CHECKS, CHECKS_FILE, 0) {
        return Err(vec![diag]);
    }
    let file: ChecksFile = parse_file(CHECKS_FILE, text, 0)?;
    let mut diags = Vec::new();
    if file.schema != crate::format::CHECKS {
        diags.push(
            schema(
                CHECKS_FILE,
                &format!("schema is '{}', which this file does not use", file.schema),
                "write 'schema: checks' with no version suffix",
            )
            .with_path("schema"),
        );
    }
    let mut checks = BTreeMap::new();
    for (index, entry) in file.checks.into_iter().enumerate() {
        let path = format!("checks[{index}]");
        let mut bad = false;
        if entry.id.is_empty() {
            diags.push(check_problem(
                text,
                index,
                "id:",
                &format!("{path}.id"),
                "a check id is empty",
                "set id to a short name",
            ));
            bad = true;
        }
        if entry.dice.count == 0 {
            diags.push(check_problem(
                text,
                index,
                "count:",
                &format!("{path}.dice.count"),
                &format!("check '{}' rolls zero dice", entry.id),
                "set dice.count to 1 or more",
            ));
            bad = true;
        }
        if entry.dice.sides == 0 {
            diags.push(check_problem(
                text,
                index,
                "sides:",
                &format!("{path}.dice.sides"),
                &format!("check '{}' has a die with zero sides", entry.id),
                "set dice.sides to 2 or more",
            ));
            bad = true;
        }
        if entry.stat.is_empty() {
            diags.push(check_problem(
                text,
                index,
                "stat:",
                &format!("{path}.stat"),
                &format!("check '{}' names no meter", entry.id),
                "set stat to a meter id from rules/meters.yaml",
            ));
            bad = true;
        }
        if !bad && checks.contains_key(&entry.id) {
            diags.push(check_problem(
                text,
                index,
                "id:",
                &format!("{path}.id"),
                &format!("check '{}' is listed twice", entry.id),
                "keep one check",
            ));
            bad = true;
        }
        if !bad {
            checks.insert(
                entry.id,
                CheckDef {
                    dice_count: entry.dice.count,
                    dice_sides: entry.dice.sides,
                    stat: entry.stat,
                },
            );
        }
    }
    if diags.is_empty() {
        Ok(checks)
    } else {
        Err(diags)
    }
}

fn check_problem(
    text: &str,
    index: usize,
    field: &str,
    path: &str,
    problem: &str,
    hint: &str,
) -> Diagnostic {
    let mut diag = schema(CHECKS_FILE, problem, hint).with_path(path);
    if let Some((line, column)) = locate_row_field(text, "id:", index, field) {
        diag = diag.at_line(line, Some(column));
    }
    diag
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
    /// `None` keeps the roll binary. An opposing total passes `None`, because
    /// `partial_at` belongs to the row's own difficulty.
    pub partial_at: Option<i64>,
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
        outcome: classify(total, roll.target, roll.partial_at),
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

    use super::{classify, load_checks, resolve_check, CheckDef, CheckOut, CheckRoll, Outcome};
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
            partial_at: None,
            rng,
            die_cursor,
        })
    }

    fn roll_band(target: i64, partial_at: Option<i64>, words: &[u32]) -> CheckOut {
        let rng = rng_of(words);
        let mut cursor = 0usize;
        let actor = npc();
        let defs = BTreeMap::from([("grit".to_string(), grit_def())]);
        let check = check();
        resolve_check(CheckRoll {
            check: &check,
            defs: &defs,
            actor_id: "mira",
            actor: &actor,
            viewer: &ViewerId::Npc("mira".to_string()),
            target,
            partial_at,
            rng: &rng,
            die_cursor: &mut cursor,
        })
        .unwrap()
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
        assert_eq!(out.outcome, Outcome::Success);
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
        assert_eq!(out.outcome, Outcome::Miss);
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

    #[test]
    fn a_band_changes_the_same_faces() {
        let binary = roll_band(9, None, &[0, 0]);
        let band = roll_band(9, Some(3), &[0, 0]);
        assert_eq!(binary.faces, vec![1, 1]);
        assert_eq!(binary.total, 3);
        assert_eq!(binary.outcome, Outcome::Miss);
        assert_eq!(band.outcome, Outcome::Partial);
        let made = roll_band(9, Some(3), &[5, 5]);
        assert_eq!(made.total, 13);
        assert_eq!(made.outcome, Outcome::Success);
    }

    fn text(tag: &str) -> String {
        format!("schema: {tag}\nchecks: []\n")
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        use crate::diagnostic::reason_of;
        assert!(load_checks(&text("checks")).is_ok());
        let old = load_checks(&text(concat!("checks/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(reason_of(old), Reason::Schema);
        let bogus = load_checks(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(reason_of(bogus), Reason::Schema);
    }

    #[test]
    fn a_check_refusal_names_the_fix() {
        let duplicate = "\
schema: checks
checks:
  - id: grit
    dice:
      count: 2
      sides: 6
    stat: grit
  - id: grit
    dice:
      count: 1
      sides: 6
    stat: grit
";
        let err = load_checks(duplicate).unwrap_err();
        assert!(
            err.iter().any(|diag| {
                diag.message.contains("listed twice")
                    && diag.path.as_deref() == Some("checks[1].id")
                    && diag.line.is_some()
            }),
            "{err:?}"
        );
        let zero = "\
schema: checks
checks:
  - id: grit
    dice:
      count: 0
      sides: 0
    stat: grit
";
        let err = load_checks(zero).unwrap_err();
        assert!(err.iter().any(|diag| diag.message.contains("zero dice")));
        assert!(err.iter().any(|diag| diag.message.contains("zero sides")));
    }
}
