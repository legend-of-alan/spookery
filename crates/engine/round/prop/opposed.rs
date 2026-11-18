//! A refused or dropped opposing roll replays the same way.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ecs::component::{ControlType, Meter};
use crate::ecs::id::ActorId;
use crate::ecs::Presence;
use crate::intent::{Intent, Source, Target};
use crate::rules::gate;
use crate::session;
use crate::store::journal;
use crate::store::records::{Record, Step};
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
        // A failed gate at the start: Harbormouse holds no chock.
        (
            None,
            "    if: [has_item: { subject: self, id: chock }]\n",
            Some(Reason::MissingItem),
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

/// The docks pack with one round `limit` line set, the lunge plus `rule`, Briar's might at 30,
/// and Mira's door note not due, so the fight is all the round holds.
fn lunge_pack(limit: Option<&str>, rule: &str) -> PathBuf {
    let pack = copy_pack("docks", "opposed-base");
    if let Some(limit) = limit {
        let (key, _) = limit.split_once(':').expect("a `key: value` line");
        let path = pack.join("rules/budget.yaml");
        let text = fs::read_to_string(&path).unwrap();
        let old = text.lines().find(|line| line.starts_with(key));
        fs::write(&path, text.replacen(old.expect("limit key"), limit, 1)).unwrap();
    }
    crate::testing::append(&pack.join("rules/verbs.yaml"), &format!("{LUNGE}{rule}"));
    crate::testing::edit_save(&pack, |world| {
        let briar = ActorId::from("briar");
        let held = world.actors().get::<Meter>(&briar).get("might").cloned();
        let might = Meter {
            base: 30,
            ..held.expect("Briar's might")
        };
        let mut actors = world.actors_mut();
        actors
            .get_mut::<Meter>(&briar)
            .set("might", Some(might))
            .unwrap();
        let shop_day = Meter { base: 0, offset: 0 };
        actors
            .get_mut::<Meter>(&ActorId::from("mira"))
            .set("shop_day", Some(shop_day))
            .unwrap();
    });
    pack
}

/// After `warmup` empty rounds, Briar strikes Harbormouse and Harbormouse lunges at Briar.
/// `refused` is the lunge's refusal, or `None` when the strike is cut from the round and
/// refused `BUDGET`.
fn fight(base: &Path, warmup: u32, refused: Option<Reason>) -> Result<(), String> {
    let pack = TempPack(fresh_dir("opposed"));
    copy_dir(base, &pack.0);
    let fail = |what: &str, detail: String| format!("warm-up {warmup}: {what}: {detail}");
    let lock = crate::lock_pack(&pack.0).map_err(|err| fail("lock", err.to_string()))?;
    let mut session =
        session::Session::resume(&pack.0, lock).map_err(|err| fail("resume", err.to_string()))?;
    let harbormouse = Presence {
        players: 1,
        actors_present: vec!["harbormouse".into()],
    };
    let wall = session.world().last_round_at.clone();
    for _ in 0..warmup {
        session
            .play(&harbormouse, &wall)
            .map_err(|err| fail("warm-up", err.to_string()))?;
    }
    // Queued past the queue's gate, as if the world changed before the start, so the start's
    // gate is what refuses the lunge.
    let round_index = session.world().round_index + 1;
    for (id, actor, verb, target) in [
        ("briar-strike", "briar", "strike", "harbormouse"),
        ("harbormouse-lunge", "harbormouse", "lunge", "briar"),
    ] {
        let viewer = (session.world().actors())
            .get::<ControlType>(&actor.into())
            .viewer();
        let target = Target::Actor(target.into());
        let intent = Intent::new(
            id.to_string(),
            round_index,
            viewer,
            Source::Gm,
            verb,
            target,
        );
        session.queue_intent(intent);
    }
    session
        .play(&harbormouse, &wall)
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
            .any(|rejected| rejected.id == "harbormouse-lunge" && rejected.code == code),
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
    drop(session);
    session::Session::resume(
        &pack.0,
        crate::lock_pack(&pack.0).map_err(|err| fail("lock", err.to_string()))?,
    )
    .map_err(|err| fail("resume", err.to_string()))?;
    Ok(())
}
