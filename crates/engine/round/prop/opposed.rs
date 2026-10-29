//! A rejected opposing roll replays the same way.

use std::fs;
use std::path::{Path, PathBuf};

use crate::journal::{self, JournalEntry};
use crate::model::Presence;
use crate::world::{GmIntentSpec, World};
use crate::Reason;

use super::play::TempPack;
use crate::testing::{copy_dir, copy_pack, fresh_dir};

const LUNGE: &str = "
  - id: lunge
    when: { verb: lunge, target: actor }
    roll: { check: strike, target: 38, partial_at: 32 }
    success: [meter: { who: target, key: hp, add: -1 }, meter: { who: target, key: hp, add: -1 }]
    partial: [meter: { who: target, key: hp, add: -1 }, meter: { who: target, key: hp, add: -1 }]
    failure: [meter: { who: self, key: hp, add: -1 }, meter: { who: self, key: hp, add: -1 }]
";

#[test]
fn a_rejected_opposing_roll_replays_the_same_way() {
    let base = TempPack(lunge_pack());
    let failures: Vec<String> = (0..12)
        .filter_map(|warmup| fight(&base.0, warmup).err())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The d20 pack with one effect per intent, a two-effect lunge, and Briar's might at 30.
fn lunge_pack() -> PathBuf {
    let pack = copy_pack("sample-d20", "opposed-base");
    let tick = pack.join("rules/tick.yaml");
    let text = fs::read_to_string(&tick).unwrap();
    let capped = text.replacen("max_effects_per_intent: 4", "max_effects_per_intent: 1", 1);
    assert_ne!(capped, text, "the effect cap moved");
    fs::write(&tick, capped).unwrap();
    crate::testing::append(&pack.join("rules/verbs.yaml"), LUNGE);
    crate::testing::edit_save(&pack, |now| {
        let briar = now.actors.get_mut("npc_briar").expect("Briar");
        briar.meters_mut().get_mut("might").expect("might").base = 30;
    });
    pack
}

/// After `warmup` empty rounds, Lark lunges at Briar while Briar strikes Lark.
fn fight(base: &Path, warmup: u32) -> Result<(), String> {
    let pack = TempPack(fresh_dir("opposed"));
    copy_dir(base, &pack.0);
    let fail = |what: &str, detail: String| format!("warm-up {warmup}: {what}: {detail}");
    let lock = crate::lock_pack(&pack.0).map_err(|err| fail("lock", err.to_string()))?;
    let mut world = World::resume(&pack.0, &lock).map_err(|err| fail("resume", err.to_string()))?;
    world.set_presence_inner(Presence {
        humans: 1,
        actors_present: vec!["pc_lark".to_string()],
    });
    let wall = world.now().last_round_at.clone();
    for _ in 0..warmup {
        world
            .play(&wall)
            .map_err(|err| fail("warm-up", err.to_string()))?;
    }
    for (id, actor, verb, target) in [
        ("lark-lunge", "pc_lark", "lunge", "npc_briar"),
        ("briar-strike", "npc_briar", "strike", "pc_lark"),
    ] {
        let spec = GmIntentSpec {
            id: Some(id.to_string()),
            round_index: None,
            actor_id: Some(actor.to_string()),
            move_name: Some(verb.to_string()),
            target: Some(target.to_string()),
            message_to: None,
            rationale_code: String::new(),
        };
        world
            .queue_gm_intent(spec, || Err(Reason::Schema))
            .map_err(|err| fail(id, err.to_string()))?;
    }
    world
        .play(&wall)
        .map_err(|err| fail("fight", err.to_string()))?;
    let entries = journal::read_entries(&pack.0).map_err(|fault| fail("journal", fault.detail))?;
    let Some(JournalEntry::Round(last)) = entries.iter().rev().find(|entry| entry.changes_state())
    else {
        return Err(fail("journal", "no round record".to_string()));
    };
    assert!(
        last.rejected
            .iter()
            .any(|rejected| rejected.id == "lark-lunge" && rejected.code == Reason::Budget),
        "warm-up {warmup}: the lunge was not rejected for its budget: {last:?}"
    );
    crate::round::verify_pack(&pack.0, None).map_err(|err| fail("verify", err.to_string()))?;
    crate::round::replay::open_at_tip(&pack.0).map_err(|err| fail("resume", err.to_string()))?;
    Ok(())
}
