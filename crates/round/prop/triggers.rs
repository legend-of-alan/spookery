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

use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestRunner};
use spookery_kernel::api::effect::EffectKind;
use spookery_kernel::api::journal::{self, JournalEntry, TickRecord};
use spookery_kernel::api::world::World;

use super::play::{configure_runner, copy_docks, pick_rounds, play_rounds, TempPack};
use crate::from_journal;

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
            let pack = TempPack(copy_docks("prop-triggers"));
            let rules = install(&pack.0, &chain).map_err(fail_msg)?;
            play_rounds(&pack.0, &rounds).map_err(fail_msg)?;
            triggers_stop(&pack.0, &rules, rounds.len())
        })
        .unwrap_or_else(|err| panic!("triggers property failed: {err}"));
}

fn pick_chain() -> impl Strategy<Value = Chain> {
    (2..=4usize, prop::collection::vec(0..4usize, 0..=2)).prop_map(|(len, extras)| Chain {
        len,
        extras: extras.into_iter().map(|watch| watch % len).collect(),
    })
}

fn triggers_stop(pack: &Path, rules: &[RuleSpec], rounds: usize) -> Result<(), TestCaseError> {
    let max = World::open(pack)
        .map_err(|err| fail_msg(err.to_string()))?
        .budget()
        .max_trigger_depth;
    let entries = journal::read_entries(pack)
        .map_err(|fault| fail_msg(format!("{}: {}", fault.file, fault.detail)))?;
    let ticks: Vec<&TickRecord> = entries
        .iter()
        .filter_map(|entry| match entry {
            JournalEntry::Tick(tick) => Some(tick),
            _ => None,
        })
        .collect();
    if ticks.len() != rounds {
        return Err(fail_msg(format!(
            "played {rounds} rounds but the journal has {} ticks",
            ticks.len()
        )));
    }
    let mut held = BTreeSet::new();
    for tick in ticks {
        held = check_tick(tick, rules, max, held, rowan_gained_oil(tick))?;
    }
    same_save(pack)
}

/// Trigger watches see the net write. Dropping the oil and gaining it back
/// leaves no change, so the chain does not start.
fn rowan_gained_oil(tick: &TickRecord) -> bool {
    let net: i64 = tick
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

fn check_tick(
    tick: &TickRecord,
    rules: &[RuleSpec],
    max: u64,
    mut held: BTreeSet<String>,
    mut oil: bool,
) -> Result<BTreeSet<String>, TestCaseError> {
    let logged: Vec<&str> = tick
        .steps_run
        .iter()
        .filter(|step| step.starts_with("story:chain/"))
        .map(String::as_str)
        .collect();
    let hit_cap = tick.steps_run.iter().any(|step| step == "trigger_depth");
    let mut armed = BTreeSet::new();
    let mut index = 0usize;
    let mut passes = 0u64;
    loop {
        let ready = ready(rules, oil, &armed);
        if ready.is_empty() {
            if index != logged.len() {
                return Err(fail_msg(format!(
                    "tick {} logged trigger rules after the chain stopped: {}",
                    tick.tick_index,
                    logged.join(",")
                )));
            }
            if hit_cap {
                return Err(fail_msg(format!(
                    "tick {} logged trigger_depth after {passes} passes, under the cap {max}",
                    tick.tick_index
                )));
            }
            return Ok(held);
        }
        if passes >= max {
            if index != logged.len() {
                return Err(fail_msg(format!(
                    "tick {} ran more than {max} trigger passes: {}",
                    tick.tick_index,
                    logged.join(",")
                )));
            }
            if !hit_cap {
                return Err(fail_msg(format!(
                    "tick {} hit the cap after {max} trigger passes without trigger_depth: {}",
                    tick.tick_index,
                    tick.steps_run.join(",")
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
                    "tick {} stopped during pass {} before {}",
                    tick.tick_index,
                    passes + 1,
                    rule.id
                )));
            };
            if *got != rule.id {
                return Err(fail_msg(format!(
                    "tick {} pass {} expected {} but logged {got}",
                    tick.tick_index,
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

fn ready<'a>(rules: &'a [RuleSpec], oil: bool, armed: &BTreeSet<String>) -> Vec<&'a RuleSpec> {
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
    if !(2..=4).contains(&chain.len) {
        return Err(format!("cycle length {} is out of range", chain.len));
    }
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
    let mut listed = String::from("  flags:\n");
    for flag in flags {
        listed.push_str(&format!("    - {flag}\n"));
    }
    let needle = "tick:\n  eligible: false\n";
    if !text.contains(needle) {
        return Err("pc_rowan tick block was not where the property expected".to_string());
    }
    let replacement = format!("tick:\n  eligible: false\n{listed}");
    fs::write(&path, text.replacen(needle, &replacement, 1)).map_err(|err| err.to_string())
}

fn same_save(pack: &Path) -> Result<(), TestCaseError> {
    let report = from_journal(pack, None, None).map_err(|err| fail_msg(err.to_string()))?;
    let entries = journal::read_entries(pack)
        .map_err(|fault| fail_msg(format!("{}: {}", fault.file, fault.detail)))?;
    let Some(last) = entries.iter().rev().find(|entry| entry.changes_state()) else {
        return Err(fail_msg("the rounds wrote no record".to_string()));
    };
    let replayed = journal::hash_now(&report.now_yaml);
    if replayed == last.now_hash() {
        return Ok(());
    }
    Err(fail_msg(format!(
        "state hash differed: record {} replayed {replayed}",
        last.now_hash()
    )))
}

fn fail_msg(message: String) -> TestCaseError {
    TestCaseError::fail(message)
}
