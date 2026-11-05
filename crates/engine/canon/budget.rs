//! Round budgets from `rules/budget.yaml`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::source::{load_tagged, FileText, Locator, Problem, BUDGET_SCHEMA};
use crate::diagnostic::Diagnostic;

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
    /// Each named cycle's length in ticks. `cycle` conditions read it.
    pub cycles: BTreeMap<String, u64>,
    pub round_every_real_seconds: u64,
    /// Empty-table wakes. `false` leaves an empty table still.
    pub idle_rounds: bool,
    pub idle_ticks_per_round: u64,
    pub live_ticks_per_round: u64,
    pub idle_catchup_max_rounds: u64,
    pub idle_max_ticks_per_real_hour: u64,
    pub max_fastforward_rounds: u64,
    /// Trigger rounds in one tick. The next round logs `trigger_depth` and does not run.
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

pub(crate) fn load_budget(rel: &str, text: &str) -> Result<Budget, Vec<Diagnostic>> {
    let doc: BudgetDoc = load_tagged(rel, text, BUDGET_SCHEMA, 0)?;
    let source = FileText {
        file: rel,
        text,
        line_base: 0,
    };
    let mut diags = Vec::new();
    let mut budget = Budget::default();
    // Each row is a counter key and the least value it allows, in report order.
    macro_rules! set_numbers {
        ($($key:ident >= $least:literal),* $(,)?) => {$(
            set_number(&source, stringify!($key), doc.$key, &mut budget.$key, $least, &mut diags);
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
    for (name, length) in doc.cycles.unwrap_or_default() {
        match u64::try_from(length) {
            Ok(ticks) if ticks > 0 => {
                budget.cycles.insert(name, ticks);
            }
            _ => diags.push(
                Problem {
                    source: &source,
                    path: &format!("cycles.{name}"),
                    at: Locator::Key(&format!("{name}:")),
                    message: format!("cycle {name} is {length}, which is not a positive length"),
                    hint: "give the cycle a length of one tick or more",
                }
                .into_diagnostic(),
            ),
        }
    }
    budget.idle_rounds = parse_idle(&source, doc.idle, &mut diags);
    set_numbers!(
        idle_ticks_per_round >= 1,
        live_ticks_per_round >= 0,
        idle_catchup_max_rounds >= 0,
        idle_max_ticks_per_real_hour >= 0,
        max_fastforward_rounds >= 0,
        max_trigger_depth >= 0,
    );
    if !diags.is_empty() {
        return Err(diags);
    }
    Ok(budget)
}

/// `value` into `slot` when it is at least `least`; otherwise `slot` keeps its default.
fn set_number(
    source: &FileText,
    key: &str,
    value: Option<i64>,
    slot: &mut u64,
    least: i64,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(n) = value else { return };
    if n >= least {
        *slot = n as u64;
        return;
    }
    let problem = if n < 0 {
        format!("{key} is {n}, which is below zero")
    } else {
        format!("{key} is 0, which is not a positive interval")
    };
    let hint = match least {
        0 => "use a number that is zero or greater",
        _ => "use a number greater than zero",
    };
    diags.push(
        Problem {
            source,
            path: key,
            at: Locator::Key(&format!("{key}:")),
            message: problem,
            hint,
        }
        .into_diagnostic(),
    );
}

fn parse_idle(source: &FileText, value: Option<String>, diags: &mut Vec<Diagnostic>) -> bool {
    match value.as_deref() {
        None | Some("on") => true,
        Some("off") => false,
        Some(other) => {
            diags.push(
                Problem {
                    source,
                    path: "idle",
                    at: Locator::Key("idle:"),
                    message: format!("idle is '{other}', which is not on or off"),
                    hint: "write idle: on or idle: off",
                }
                .into_diagnostic(),
            );
            true
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetDoc {
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
    #[serde(default, deserialize_with = "read_idle_word")]
    idle: Option<String>,
    idle_ticks_per_round: Option<i64>,
    live_ticks_per_round: Option<i64>,
    idle_catchup_max_rounds: Option<i64>,
    idle_max_ticks_per_real_hour: Option<i64>,
    max_fastforward_rounds: Option<i64>,
    max_trigger_depth: Option<i64>,
}

fn read_idle_word<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
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

    fn load_budget(text: &str) -> Result<Budget, Vec<Diagnostic>> {
        super::load_budget("rules/budget.yaml", text)
    }

    fn text(tag: &str) -> String {
        format!("schema: {tag}\n")
    }

    #[test]
    fn a_negative_budget_names_the_field() {
        let text = "schema: budget\nmax_npcs_acting: -1\nidle_catchup_max_rounds: -3\n";
        let err = load_budget(text).unwrap_err();
        assert!(err
            .iter()
            .any(|diag| diag.message.contains("max_npcs_acting") && diag.line.is_some()));
        assert!(err
            .iter()
            .any(|diag| diag.message.contains("idle_catchup_max_rounds")));
    }

    #[test]
    fn the_removed_live_tick_key_is_refused() {
        use crate::diagnostic::first_reason;
        let mut stale = text("budget");
        stale.push_str("live_tick_every_real_seconds: 60\n");
        let err = load_budget(&stale).unwrap_err();
        assert_eq!(first_reason(&err), Reason::Schema);
    }

    #[test]
    fn idle_defaults_to_on_and_off_is_loaded() {
        let on = load_budget(&text("budget")).unwrap();
        assert!(on.idle_rounds);
        let mut explicit = text("budget");
        explicit.push_str("idle: on\n");
        assert!(load_budget(&explicit).unwrap().idle_rounds);
        let mut off = text("budget");
        off.push_str("idle: off\n");
        assert!(!load_budget(&off).unwrap().idle_rounds);
    }

    #[test]
    fn an_empty_idle_word_is_schema() {
        use crate::diagnostic::first_reason;
        let mut empty = text("budget");
        empty.push_str("idle:\n");
        let err = load_budget(&empty).unwrap_err();
        assert_eq!(first_reason(&err), Reason::Schema);
    }

    #[test]
    fn docks_budget_leaves_idle_on() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/rules/budget.yaml");
        let text = std::fs::read_to_string(path).unwrap();
        let budget = load_budget(&text).unwrap();
        assert!(budget.idle_rounds);
        assert_eq!(budget.round_every_real_seconds, 300);
        assert_eq!(budget.idle_ticks_per_round, 60);
    }
}
