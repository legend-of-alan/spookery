//! Round budgets from `rules/budget.yaml`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::refusal::{Diagnostics, FileText, Locator};
use crate::diagnostic::{parse_tagged, Diagnostic};

const SCHEMA: &str = "budget";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_intents_applied: u64,
    pub max_npcs_acting: u64,
    pub max_effects_per_intent: u64,
    pub max_state_keys_mutated: u64,
    pub max_memory_writes: u64,
    pub idle_acts_per_window: u64,
    /// Ticks in one idle act window. `idle_acts_per_window` resets when a window ends.
    pub idle_act_window_ticks: u64,
    pub live_acts_per_window: u64,
    /// Ticks in one live act window. `live_acts_per_window` resets when a window ends.
    pub live_act_window_ticks: u64,
    /// Each cycle's length in ticks, by its id. `cycle` conds read it.
    pub cycles: BTreeMap<String, u64>,
    pub round_every_real_seconds: u64,
    /// Empty-table wakes. `false` leaves an empty table still.
    pub idle_rounds: bool,
    pub idle_ticks_per_round: u64,
    pub live_ticks_per_round: u64,
    pub idle_catchup_max_rounds: u64,
    pub idle_max_ticks_per_real_hour: u64,
    pub max_fastforward_rounds: u64,
    /// Passes of change rules after one action lands. The next pass logs `trigger_depth` and
    /// does not run.
    pub max_trigger_depth: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_intents_applied: 12,
            max_npcs_acting: 8,
            max_effects_per_intent: 4,
            max_state_keys_mutated: 24,
            max_memory_writes: 8,
            idle_acts_per_window: 3,
            idle_act_window_ticks: 1440,
            live_acts_per_window: 6,
            live_act_window_ticks: 60,
            cycles: BTreeMap::new(),
            round_every_real_seconds: 300,
            idle_rounds: true,
            idle_ticks_per_round: 60,
            live_ticks_per_round: 5,
            idle_catchup_max_rounds: 12,
            idle_max_ticks_per_real_hour: 480,
            max_fastforward_rounds: 12,
            max_trigger_depth: 4,
        }
    }
}

pub(super) fn load(file: &str, text: &str) -> Result<Budget, Vec<Diagnostic>> {
    let yaml: Yaml = parse_tagged(file, text, SCHEMA, false)?;
    let file_text = FileText { file, text };
    let mut diagnostics = Diagnostics::default();
    let mut budget = Budget::default();
    macro_rules! set_numbers {
        ($($key:ident >= $least:literal),* $(,)?) => {$(
            set_number(&file_text, stringify!($key), yaml.$key, &mut budget.$key, $least, &mut diagnostics);
        )*};
    }
    set_numbers!(
        max_intents_applied >= 0,
        max_npcs_acting >= 0,
        max_effects_per_intent >= 0,
        max_state_keys_mutated >= 0,
        max_memory_writes >= 0,
        idle_acts_per_window >= 0,
        idle_act_window_ticks >= 1,
        live_acts_per_window >= 0,
        live_act_window_ticks >= 1,
        round_every_real_seconds >= 1,
    );
    for (id, length) in yaml.cycles.unwrap_or_default() {
        match u64::try_from(length) {
            Ok(ticks) if ticks > 0 => {
                budget.cycles.insert(id, ticks);
            }
            _ => diagnostics.push(file_text.refuse_at(
                &format!("cycles.{id}"),
                Locator::Key(&format!("{id}:")),
                format!("cycle {id} is {length}, which is not a positive length"),
                "give the cycle a length of one tick or more",
            )),
        }
    }
    budget.idle_rounds = parse_idle(&file_text, yaml.idle, &mut diagnostics);
    set_numbers!(
        idle_ticks_per_round >= 1,
        live_ticks_per_round >= 0,
        idle_catchup_max_rounds >= 0,
        idle_max_ticks_per_real_hour >= 0,
        max_fastforward_rounds >= 0,
        max_trigger_depth >= 0,
    );
    diagnostics.into_result(budget)
}

/// `value` into `slot` when it is at least `least`; otherwise `slot` keeps its default.
fn set_number(
    file_text: &FileText,
    key: &str,
    value: Option<i64>,
    slot: &mut u64,
    least: i64,
    diagnostics: &mut Diagnostics,
) {
    let Some(n) = value else { return };
    if n >= least {
        *slot = n as u64;
        return;
    }
    let message = if n < 0 {
        format!("{key} is {n}, which is below zero")
    } else {
        format!("{key} is 0, which is not a positive interval")
    };
    let hint = match least {
        0 => "use a number that is zero or greater",
        _ => "use a number greater than zero",
    };
    diagnostics.push(file_text.refuse_at(key, Locator::Key(&format!("{key}:")), message, hint));
}

fn parse_idle(file_text: &FileText, value: Option<String>, diagnostics: &mut Diagnostics) -> bool {
    match value.as_deref() {
        None | Some("on") => true,
        Some("off") => false,
        Some(other) => {
            diagnostics.push(file_text.refuse_at(
                "idle",
                Locator::Key("idle:"),
                format!("idle is '{other}', which is not on or off"),
                "write idle: on or idle: off",
            ));
            true
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Yaml {
    #[serde(rename = "schema")]
    _schema: String,
    max_intents_applied: Option<i64>,
    max_npcs_acting: Option<i64>,
    max_effects_per_intent: Option<i64>,
    max_state_keys_mutated: Option<i64>,
    max_memory_writes: Option<i64>,
    idle_acts_per_window: Option<i64>,
    idle_act_window_ticks: Option<i64>,
    live_acts_per_window: Option<i64>,
    live_act_window_ticks: Option<i64>,
    cycles: Option<BTreeMap<String, i64>>,
    round_every_real_seconds: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_idle_word")]
    idle: Option<String>,
    idle_ticks_per_round: Option<i64>,
    live_ticks_per_round: Option<i64>,
    idle_catchup_max_rounds: Option<i64>,
    idle_max_ticks_per_real_hour: Option<i64>,
    max_fastforward_rounds: Option<i64>,
    max_trigger_depth: Option<i64>,
}

fn deserialize_idle_word<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let word = String::deserialize(deserializer)?;
    Ok(Some(word))
}

#[cfg(test)]
mod tests {
    use super::Budget;
    use crate::diagnostic::Diagnostic;
    use crate::Reason;

    fn load(text: &str) -> Result<Budget, Vec<Diagnostic>> {
        super::load("rules/budget.yaml", text)
    }

    fn text(tag: &str) -> String {
        format!("schema: {tag}\n")
    }

    #[test]
    fn a_negative_budget_names_the_field() {
        let text = "schema: budget\nmax_npcs_acting: -1\nidle_catchup_max_rounds: -3\n";
        let err = load(text).unwrap_err();
        assert!(err
            .iter()
            .any(|diagnostic| diagnostic.message.contains("max_npcs_acting")
                && diagnostic.line.is_some()));
        assert!(err
            .iter()
            .any(|diagnostic| diagnostic.message.contains("idle_catchup_max_rounds")));
    }

    #[test]
    fn idle_defaults_to_on_and_off_is_loaded() {
        let on = load(&text("budget")).unwrap();
        assert!(on.idle_rounds);
        let mut explicit = text("budget");
        explicit.push_str("idle: on\n");
        assert!(load(&explicit).unwrap().idle_rounds);
        let mut off = text("budget");
        off.push_str("idle: off\n");
        assert!(!load(&off).unwrap().idle_rounds);
    }

    #[test]
    fn an_empty_idle_word_is_schema() {
        use crate::diagnostic::first_reason;
        let mut empty = text("budget");
        empty.push_str("idle:\n");
        let err = load(&empty).unwrap_err();
        assert_eq!(first_reason(&err), Reason::Schema);
    }
}
