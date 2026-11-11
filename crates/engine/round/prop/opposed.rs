//! A refused or dropped opposing roll replays the same way.

use std::fs;
use std::path::{Path, PathBuf};

use crate::intent::{Intent, Source, Target};
use crate::rules::gate;
use crate::state::id::ActorId;
use crate::state::Presence;
use crate::store::journal;
use crate::store::records::{Record, Step};
use crate::world::World;
use crate::Reason;

use super::support::TempPack;
use crate::testing::{copy_dir, copy_pack, fresh_dir};

const LUNGE: &str = "
  - id: lunge
    when: { verb: lunge, target: actor }
    roll: { check: strike, threshold: 38, partial_at: 32 }
    success: [meter: { of: target, key: hp, add: -1 }, meter: { of: target, key: hp, add: -1 }]
    partial: [meter: { of: target, key: hp, add: -1 }, meter: { of: target, key: hp, add: -1 }]
    miss: [meter: { of: self, key: hp, add: -1 }, meter: { of: self, key: hp, add: -1 }]
";

#[test]
fn a_refused_or_dropped_opposing_roll_replays_the_same_way() {
    let cases = [
        // Two effects over a cap of one.
        (Some("max_effects_per_intent: 1"), "", Some(Reason::Budget)),
        // A failed gate at the start: Lark holds no chock.
        (
            None,
            "    if: [has_item: { subject: self, id: chock }]\n",
            Some(Reason::MissingItem),
        ),
        // A dice cost passes the gate, then goes unpaid in apply.
        (
            None,
            "    cost: { items: { chock: 1d4 } }\n",
            Some(Reason::CantAfford),
        ),
        // Players go first, so the strike is cut, refused `BUDGET`, and the lunge reads it from
        // the raw queue.
        (Some("max_intents_applied: 1"), "", None),
    ];
    let mut failures = Vec::new();
    for (limit, rule, refused) in cases {
        let base = TempPack(lunge_pack(limit, rule));
        failures.extend((0..12).filter_map(|warmup| fight(&base.0, warmup, refused).err()));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The d20 pack with one round `limit` line set, the lunge plus `rule`, and Briar's might at 30.
fn lunge_pack(limit: Option<&str>, rule: &str) -> PathBuf {
    let pack = copy_pack("sample-d20", "opposed-base");
    if let Some(limit) = limit {
        let (key, _) = limit.split_once(':').expect("a `key: value` line");
        let path = pack.join("rules/budget.yaml");
        let text = fs::read_to_string(&path).unwrap();
        let old = text.lines().find(|line| line.starts_with(key));
        fs::write(&path, text.replacen(old.expect("limit key"), limit, 1)).unwrap();
    }
    crate::testing::append(&pack.join("rules/verbs.yaml"), &format!("{LUNGE}{rule}"));
    crate::testing::edit_save_with(&pack, |state, canon| {
        let briar = ActorId::from("briar");
        let held = state
            .actors()
            .meter(&briar, "might")
            .expect("Briar's might");
        let might = crate::state::component::meter::Meter {
            base: 30,
            ..held.clone()
        };
        let mut actors = state.actors_mut();
        actors
            .set_meter(canon, &briar, "might", Some(might))
            .unwrap();
    });
    pack
}

/// After `warmup` empty rounds, Briar strikes Lark and Lark lunges at Briar.
/// `refused` is the lunge's refusal, or `None` when the strike is cut from the round and
/// refused `BUDGET`.
fn fight(base: &Path, warmup: u32, refused: Option<Reason>) -> Result<(), String> {
    let pack = TempPack(fresh_dir("opposed"));
    copy_dir(base, &pack.0);
    let fail = |what: &str, detail: String| format!("warm-up {warmup}: {what}: {detail}");
    let lock = crate::lock_pack(&pack.0).map_err(|err| fail("lock", err.to_string()))?;
    let mut world = World::resume(&pack.0, lock).map_err(|err| fail("resume", err.to_string()))?;
    let lark = Presence {
        players: 1,
        actors_present: vec!["lark".into()],
    };
    let wall = world.state().last_round_at.clone();
    for _ in 0..warmup {
        world
            .play(&lark, &wall)
            .map_err(|err| fail("warm-up", err.to_string()))?;
    }
    // Queued past the queue's gate, as if the world changed before the start, so the start's
    // gate is what refuses the lunge.
    let round_index = world.state().round_index + 1;
    for (id, actor, verb, target) in [
        ("briar-strike", "briar", "strike", "lark"),
        ("lark-lunge", "lark", "lunge", "briar"),
    ] {
        let viewer = world.state().viewer_of(&actor.into());
        let target = Target::Actor(target.into());
        let intent = Intent::new(
            id.to_string(),
            round_index,
            viewer,
            Source::Gm,
            verb,
            target,
        );
        world.queue_intent(intent);
    }
    world
        .play(&lark, &wall)
        .map_err(|err| fail("fight", err.to_string()))?;
    let entries = journal::read_records(&pack.0).map_err(|fault| fail("journal", fault.detail))?;
    let Some(Record::Round(last)) = entries.iter().rev().find(|entry| entry.changes_state()) else {
        return Err(fail("journal", "no round record".to_string()));
    };
    let ran = |rule: &str| last.extra_steps.contains(&Step::Rule(rule.to_string()));
    let as_expected = match refused {
        Some(code) => last
            .refused
            .iter()
            .any(|rejected| rejected.id == "lark-lunge" && rejected.code == code),
        None => {
            let cut = gate::Refusal {
                id: "briar-strike".to_string(),
                code: Reason::Budget,
                detail: None,
            };
            last.refused == [cut] && ran("verb:lunge") && !ran("verb:strike")
        }
    };
    assert!(
        as_expected,
        "warm-up {warmup}: not refused with {refused:?}: {last:?}"
    );
    crate::round::verify_pack(&pack.0, None).map_err(|err| fail("verify", err.to_string()))?;
    drop(world);
    World::resume(
        &pack.0,
        crate::lock_pack(&pack.0).map_err(|err| fail("lock", err.to_string()))?,
    )
    .map_err(|err| fail("resume", err.to_string()))?;
    Ok(())
}
