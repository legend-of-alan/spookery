//! The round crate has no cucumber harness. This is the scenario:
//!
//! ```gherkin
//! Feature: Triggers stop at the cap
//!
//!   Scenario: Random play stops trigger chains at the cap
//!     Given the docks pack
//!     And random change rules, including rules that set each other's watched values
//!     When random legal player moves and habit rounds are played
//!     Then every action is followed by at most max_trigger_depth trigger passes
//!     And a round that hit the cap logs trigger_depth
//!     And replaying the journal yields the state hash stored on the last record
//! ```
//!
//! The chain lives on rowan. It starts after each of that PC's actions that
//! leaves its lamp oil count higher than before the action, and runs to the cap
//! before the next action. Other PCs are not given the flags, so a fire for
//! them rolls back and the log has one subject.
//!
//! Deterministic in CI: 64 cases, seed `0x0018_0018`, no failure file.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use crate::rules::effect::EffectKind;
use crate::store::journal;
use crate::store::records::{Record, RoundRecord, Step};
use crate::world::World;
use proptest::prelude::*;
use proptest::test_runner::{TestCaseError, TestRunner};

use super::support::{arb_rounds, assert_replay_matches, play_rounds, runner_config, TempPack};
use crate::testing::copy_docks;

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
    let mut runner = TestRunner::new(runner_config());
    let rounds = arb_rounds();
    let chain = pick_chain();
    runner
        .run(&(rounds, chain), |(rounds, chain)| {
            let pack = TempPack(copy_docks("prop-triggers"));
            let rules = install(&pack.0, &chain).map_err(TestCaseError::fail)?;
            play_rounds(&pack.0, &rounds).map_err(TestCaseError::fail)?;
            assert_triggers_stop(&pack.0, &rules, rounds.len())
        })
        .unwrap_or_else(|err| panic!("triggers property failed: {err}"));
}

fn pick_chain() -> impl Strategy<Value = Chain> {
    (2..=4usize).prop_flat_map(|len| {
        prop::collection::vec(0..len, 0..=2).prop_map(move |extras| Chain { len, extras })
    })
}

fn assert_triggers_stop(
    pack: &Path,
    rules: &[RuleSpec],
    rounds: usize,
) -> Result<(), TestCaseError> {
    let max = World::open(pack)
        .map_err(|err| TestCaseError::fail(err.to_string()))?
        .budget()
        .max_trigger_depth;
    let entries = journal::read_records(pack)
        .map_err(|fault| TestCaseError::fail(format!("{}: {}", fault.file, fault.detail)))?;
    let records: Vec<&RoundRecord> = entries
        .iter()
        .filter_map(|entry| match entry {
            Record::Round(record) => Some(record),
            _ => None,
        })
        .collect();
    if records.len() != rounds {
        return Err(TestCaseError::fail(format!(
            "played {rounds} rounds but the journal has {} round records",
            records.len()
        )));
    }
    let mut held = BTreeSet::new();
    for record in records {
        held = check_round(record, rules, max, held, rowan_oil_gains(record))?;
    }
    assert_replay_matches(pack)
}

/// How many of the round's actions left rowan with more lamp oil, in order. A trigger pass
/// runs after each action and sees that action's net write.
fn rowan_oil_gains(record: &RoundRecord) -> usize {
    let mut nets: Vec<(&str, i64)> = Vec::new();
    for effect in &record.effects {
        let EffectKind::Item {
            actor_id,
            place,
            id,
            delta,
            ..
        } = &effect.kind
        else {
            continue;
        };
        if place.is_some() || *actor_id != Some("rowan".into()) || id != "lamp_oil" {
            continue;
        }
        match nets
            .iter_mut()
            .find(|(intent, _)| *intent == effect.intent_id)
        {
            Some((_, net)) => *net += delta,
            None => nets.push((&effect.intent_id, *delta)),
        }
    }
    nets.iter().filter(|(_, net)| *net > 0).count()
}

fn check_round(
    record: &RoundRecord,
    rules: &[RuleSpec],
    max: u64,
    mut held: BTreeSet<String>,
    starts: usize,
) -> Result<BTreeSet<String>, TestCaseError> {
    let logged: Vec<&str> = record
        .extra_steps
        .iter()
        .filter_map(|step| match step {
            Step::Rule(id) if id.starts_with("story:chain/") => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut index = 0usize;
    let mut capped = false;
    for _ in 0..starts {
        let (next_held, hit) = check_chain(record, rules, max, held, &logged, &mut index)?;
        held = next_held;
        capped |= hit;
    }
    if index != logged.len() {
        return Err(TestCaseError::fail(format!(
            "round {} logged trigger rules after the chains stopped: {}",
            record.round_index,
            logged.join(",")
        )));
    }
    let hit_cap = record.extra_steps.contains(&Step::TriggerDepth);
    if hit_cap != capped {
        return Err(TestCaseError::fail(format!(
            "round {} logged trigger_depth: {hit_cap}, a chain hit the cap {max}: {capped}
{:?}",
            record.round_index, record.extra_steps
        )));
    }
    Ok(held)
}

/// One chain, from the oil gain that lights it, read off `logged` from `index`. Returns the
/// flags held after it and whether it hit the cap.
fn check_chain(
    record: &RoundRecord,
    rules: &[RuleSpec],
    max: u64,
    mut held: BTreeSet<String>,
    logged: &[&str],
    index: &mut usize,
) -> Result<(BTreeSet<String>, bool), TestCaseError> {
    let mut oil = true;
    let mut armed = BTreeSet::new();
    let mut passes = 0u64;
    loop {
        let ready = ready_rules(rules, oil, &armed);
        if ready.is_empty() {
            return Ok((held, false));
        }
        if passes >= max {
            return Ok((held, true));
        }
        oil = false;
        let mut next_held = held.clone();
        let mut next_armed = BTreeSet::new();
        for rule in ready {
            let Some(got) = logged.get(*index) else {
                return Err(TestCaseError::fail(format!(
                    "round {} stopped during pass {} before {}",
                    record.round_index,
                    passes + 1,
                    rule.id
                )));
            };
            if *got != rule.id {
                return Err(TestCaseError::fail(format!(
                    "round {} pass {} expected {} but logged {got}",
                    record.round_index,
                    passes + 1,
                    rule.id
                )));
            }
            *index += 1;
            apply_effects(&mut next_held, &mut next_armed, &rule.effects);
        }
        held = next_held;
        armed = next_armed;
        passes += 1;
    }
}

fn ready_rules<'a>(
    rules: &'a [RuleSpec],
    oil: bool,
    armed: &BTreeSet<String>,
) -> Vec<&'a RuleSpec> {
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
            "      - flag:\n          of: self\n          key: {key}\n          value: {value}\n"
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
        return Err("rowan idle block was not where the property expected".to_string());
    }
    let replacement = format!("idle:\n  eligible: false\n{listed}");
    fs::write(&path, text.replacen(needle, &replacement, 1)).map_err(|err| err.to_string())
}
