//! The round crate has no cucumber harness. This is the scenario:
//!
//! ```gherkin
//! Feature: Triggers stop at the cap
//!
//!   Scenario: Random play stops trigger chains at the cap
//!     Given the docks pack
//!     And random change rules, including rules that set each other's watched values
//!     When random legal player moves and habit rounds are played
//!     Then every round commits and runs at most max_trigger_depth trigger passes
//!     And a round that hit the cap logs trigger_depth
//!     And replaying the journal yields the state hash stored on the last record
//! ```
//!
//! The chain lives on pc_rowan. It starts when that PC's lamp oil count is
//! higher at the end of the round than at the start. A drop that puts the tin
//! back cancels the grit gain, and the chain does not start. Other PCs are not
//! given the flags, so a fire for them rolls back and the log has one subject.
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::effect::EffectKind;
use crate::journal::{self, JournalEntry, RoundRecord, Step};
use crate::world::World;
use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestRunner};

use super::play::{configure_runner, fail_msg, pick_rounds, play_rounds, same_save, TempPack};
use crate::testing::temp_pack;

#[derive(Clone, Debug)]
struct Chain {
    len: usize,
    extras: Vec<usize>,
}

/// `watch` is the flag that must become true. `None` watches lamp oil gained.
#[derive(Clone, Debug)]
struct RuleSpec {
    id: String,
    watch: Option<String>,
    effects: Vec<(String, bool)>,
}

#[test]
fn random_play_stops_trigger_chains_at_the_cap() {
    let mut runner = TestRunner::new(configure_runner());
    let rounds = pick_rounds();
    let chain = pick_chain();
    runner
        .run(&(rounds, chain), |(rounds, chain)| {
            let pack = TempPack(temp_pack("prop-triggers"));
            let rules = install(&pack.0, &chain).map_err(fail_msg)?;
            play_rounds(&pack.0, &rounds).map_err(fail_msg)?;
            triggers_stop(&pack.0, &rules, rounds.len())
        })
        .unwrap_or_else(|err| panic!("triggers property failed: {err}"));
}

fn pick_chain() -> impl Strategy<Value = Chain> {
    (2..=4usize).prop_flat_map(|len| {
        prop::collection::vec(0..len, 0..=2).prop_map(move |extras| Chain { len, extras })
    })
}

fn triggers_stop(pack: &Path, rules: &[RuleSpec], rounds: usize) -> Result<(), TestCaseError> {
    let max = World::open(pack)
        .map_err(|err| fail_msg(err.to_string()))?
        .budget()
        .max_trigger_depth;
    let entries = journal::read_entries(pack)
        .map_err(|fault| fail_msg(format!("{}: {}", fault.file, fault.detail)))?;
    let records: Vec<&RoundRecord> = entries
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Round(record) => Some(record),
            _ => None,
        })
        .collect();
    if records.len() != rounds {
        return Err(fail_msg(format!(
            "played {rounds} rounds but the journal has {} round records",
            records.len()
        )));
    }
    let mut held = BTreeSet::new();
    for record in records {
        held = check_round(record, rules, max, held, rowan_gained_oil(record))?;
    }
    same_save(pack)
}

/// Trigger watches see the net write. Dropping the oil and gaining it back
/// leaves no change, so the chain does not start.
fn rowan_gained_oil(record: &RoundRecord) -> bool {
    let net: i64 = record
        .effects
        .iter()
        .filter_map(|effect| match &effect.class {
            EffectKind::Item {
                actor_id,
                place,
                id,
                delta,
                ..
            } if place.is_none() && actor_id.as_deref() == Some("pc_rowan") && id == "lamp_oil" => {
                Some(*delta)
            }
            _ => None,
        })
        .sum();
    net > 0
}

fn check_round(
    record: &RoundRecord,
    rules: &[RuleSpec],
    max: u64,
    mut held: BTreeSet<String>,
    mut oil: bool,
) -> Result<BTreeSet<String>, TestCaseError> {
    let logged: Vec<&str> = record
        .steps_run
        .iter()
        .filter_map(|step| match step {
            Step::Rule(id) if id.starts_with("story:chain/") => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let hit_cap = record.steps_run.contains(&Step::TriggerDepth);
    let mut armed = BTreeSet::new();
    let mut index = 0usize;
    let mut passes = 0u64;
    loop {
        let ready = pick_ready(rules, oil, &armed);
        if ready.is_empty() {
            if index != logged.len() {
                return Err(fail_msg(format!(
                    "round {} logged trigger rules after the chain stopped: {}",
                    record.round_index,
                    logged.join(",")
                )));
            }
            if hit_cap {
                return Err(fail_msg(format!(
                    "round {} logged trigger_depth after {passes} passes, under the cap {max}",
                    record.round_index
                )));
            }
            return Ok(held);
        }
        if passes >= max {
            if index != logged.len() {
                return Err(fail_msg(format!(
                    "round {} ran more than {max} trigger passes: {}",
                    record.round_index,
                    logged.join(",")
                )));
            }
            if !hit_cap {
                return Err(fail_msg(format!(
                    "round {} hit the cap after {max} trigger passes without trigger_depth: {:?}",
                    record.round_index, record.steps_run
                )));
            }
            return Ok(held);
        }
        oil = false;
        let mut next_held = held.clone();
        let mut next_armed = BTreeSet::new();
        for rule in ready {
            let Some(got) = logged.get(index) else {
                return Err(fail_msg(format!(
                    "round {} stopped during pass {} before {}",
                    record.round_index,
                    passes + 1,
                    rule.id
                )));
            };
            if *got != rule.id {
                return Err(fail_msg(format!(
                    "round {} pass {} expected {} but logged {got}",
                    record.round_index,
                    passes + 1,
                    rule.id
                )));
            }
            index += 1;
            apply_effects(&mut next_held, &mut next_armed, &rule.effects);
        }
        held = next_held;
        armed = next_armed;
        passes += 1;
    }
}

fn pick_ready<'a>(rules: &'a [RuleSpec], oil: bool, armed: &BTreeSet<String>) -> Vec<&'a RuleSpec> {
    rules
        .iter()
        .filter(|rule| match &rule.watch {
            None => oil,
            Some(flag) => armed.contains(flag),
        })
        .collect()
}

fn apply_effects(
    held: &mut BTreeSet<String>,
    armed: &mut BTreeSet<String>,
    effects: &[(String, bool)],
) {
    for (flag, value) in effects {
        if *value {
            if held.insert(flag.clone()) {
                armed.insert(flag.clone());
            }
        } else if held.remove(flag) {
            armed.remove(flag);
        }
    }
}

fn install(pack: &Path, chain: &Chain) -> Result<Vec<RuleSpec>, String> {
    let mut flags = Vec::new();
    for index in 0..chain.len {
        flags.push(format!("c{index}"));
    }
    for index in 0..chain.extras.len() {
        flags.push(format!("s{index}"));
    }
    write_flags(pack, &flags)?;
    let mut rules = Vec::new();
    let mut yaml = String::from("schema: story\nid: chain\nrules:\n");
    push_rule(
        &mut rules,
        &mut yaml,
        "light",
        None,
        vec![("c0".to_string(), true)],
    );
    for index in 0..chain.len {
        let local = format!("c{index}");
        let next = format!("c{}", (index + 1) % chain.len);
        push_rule(
            &mut rules,
            &mut yaml,
            &local,
            Some(local.clone()),
            vec![(next, true), (local.clone(), false)],
        );
    }
    for (index, watch) in chain.extras.iter().enumerate() {
        let local = format!("x{index}");
        let side = format!("s{index}");
        let watched = format!("c{watch}");
        push_rule(
            &mut rules,
            &mut yaml,
            &local,
            Some(watched),
            vec![(side, true)],
        );
    }
    let dir = pack.join("story");
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    fs::write(dir.join("chain.yaml"), yaml).map_err(|err| err.to_string())?;
    Ok(rules)
}

fn push_rule(
    rules: &mut Vec<RuleSpec>,
    yaml: &mut String,
    local: &str,
    watch: Option<String>,
    effects: Vec<(String, bool)>,
) {
    yaml.push_str(&rule_yaml(local, watch.as_deref(), &effects));
    rules.push(RuleSpec {
        id: format!("story:chain/{local}"),
        watch,
        effects,
    });
}

fn rule_yaml(id: &str, watch: Option<&str>, effects: &[(String, bool)]) -> String {
    let when = match watch {
        Some(flag) => format!("      change:\n        flag: {flag}\n        becomes: true\n"),
        None => "      change:\n        item: lamp_oil\n        gained: true\n".to_string(),
    };
    let mut body = format!("  - id: {id}\n    when:\n{when}    do:\n");
    for (key, value) in effects {
        let value = if *value { "true" } else { "false" };
        body.push_str(&format!(
            "      - flag:\n          who: self\n          key: {key}\n          value: {value}\n"
        ));
    }
    body
}

fn write_flags(pack: &Path, flags: &[String]) -> Result<(), String> {
    let path = pack.join("cast/pc-rowan.md");
    let text = fs::read_to_string(&path).map_err(|err| err.to_string())?;
    let mut listed = String::from("flags:\n");
    for flag in flags {
        listed.push_str(&format!("    - {flag}\n"));
    }
    let needle = "idle:\n  eligible: false\n";
    if !text.contains(needle) {
        return Err("pc_rowan idle block was not where the property expected".to_string());
    }
    let replacement = format!("idle:\n  eligible: false\n{listed}");
    fs::write(&path, text.replacen(needle, &replacement, 1)).map_err(|err| err.to_string())
}
