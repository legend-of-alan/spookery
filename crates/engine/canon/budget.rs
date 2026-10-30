//! Round budgets and pause policy from `rules/budget.yaml`.

use serde::Deserialize;

use crate::canon::source::{load_tagged, FileText, Locator, Problem, BUDGET_SCHEMA};
use crate::diagnostic::Diagnostic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_intents_applied: u64,
    pub max_npcs_acting: u64,
    pub max_effects_per_intent: u64,
    pub max_state_keys_mutated: u64,
    pub max_memory_writes: u64,
    pub idle_acts_per_world_day: u64,
    pub live_acts_per_world_hour: u64,
    pub round_every_real_seconds: u64,
    /// Empty-table wakes. `false` leaves an empty table still.
    pub idle_rounds: bool,
    pub idle_minutes_per_round: u64,
    pub live_minutes_per_round: u64,
    pub idle_catchup_max_rounds: u64,
    pub idle_max_world_hours_per_real_hour: u64,
    pub max_fastforward_rounds: u64,
    /// Trigger rounds in one tick. The next round logs `trigger_depth` and does not run.
    pub max_trigger_depth: u64,
    pub pause: PausePolicies,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PausePolicies {
    pub named_plot_clocks: PausePolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum PausePolicy {
    #[serde(rename = "pause_when_empty")]
    WhenEmpty,
    #[serde(rename = "always_run")]
    AlwaysRun,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_intents_applied: 12,
            max_npcs_acting: 8,
            max_effects_per_intent: 4,
            max_state_keys_mutated: 24,
            max_memory_writes: 8,
            idle_acts_per_world_day: 3,
            live_acts_per_world_hour: 6,
            round_every_real_seconds: 300,
            idle_rounds: true,
            idle_minutes_per_round: 60,
            live_minutes_per_round: 5,
            idle_catchup_max_rounds: 12,
            idle_max_world_hours_per_real_hour: 8,
            max_fastforward_rounds: 12,
            max_trigger_depth: 4,
            pause: PausePolicies {
                named_plot_clocks: PausePolicy::WhenEmpty,
            },
        }
    }
}

pub(crate) fn load_budget(rel: &str, text: &str) -> Result<Budget, Vec<Diagnostic>> {
    let file: BudgetFile = load_tagged(rel, text, BUDGET_SCHEMA, 0)?;
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
            set_number(&source, stringify!($key), file.$key, &mut budget.$key, $least, &mut diags);
        )*};
    }
    set_numbers!(
        max_intents_applied >= 0,
        max_npcs_acting >= 0,
        max_effects_per_intent >= 0,
        max_state_keys_mutated >= 0,
        max_memory_writes >= 0,
        idle_acts_per_world_day >= 0,
        live_acts_per_world_hour >= 0,
        round_every_real_seconds >= 1,
    );
    budget.idle_rounds = parse_idle(&source, file.idle, &mut diags);
    set_numbers!(
        idle_minutes_per_round >= 1,
        live_minutes_per_round >= 0,
        idle_catchup_max_rounds >= 0,
        idle_max_world_hours_per_real_hour >= 0,
        max_fastforward_rounds >= 0,
        max_trigger_depth >= 0,
    );
    budget.pause.named_plot_clocks = file.pause.named_plot_clocks;
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
struct BudgetFile {
    #[serde(rename = "schema")]
    _schema: String,
    max_intents_applied: Option<i64>,
    max_npcs_acting: Option<i64>,
    max_effects_per_intent: Option<i64>,
    max_state_keys_mutated: Option<i64>,
    max_memory_writes: Option<i64>,
    idle_acts_per_world_day: Option<i64>,
    live_acts_per_world_hour: Option<i64>,
    round_every_real_seconds: Option<i64>,
    #[serde(default, deserialize_with = "read_idle_word")]
    idle: Option<String>,
    idle_minutes_per_round: Option<i64>,
    live_minutes_per_round: Option<i64>,
    idle_catchup_max_rounds: Option<i64>,
    idle_max_world_hours_per_real_hour: Option<i64>,
    max_fastforward_rounds: Option<i64>,
    max_trigger_depth: Option<i64>,
    pause: PauseYaml,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseYaml {
    named_plot_clocks: PausePolicy,
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
    use crate::error::Reason;

    fn load_budget(text: &str) -> Result<Budget, Vec<Diagnostic>> {
        super::load_budget("rules/budget.yaml", text)
    }

    fn text(tag: &str) -> String {
        format!("schema: {tag}\npause:\n  named_plot_clocks: pause_when_empty\n")
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        use crate::diagnostic::first_reason;
        assert!(load_budget(&text("budget")).is_ok());
        let old = load_budget(&text(concat!("budget/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(first_reason(&old), Reason::Schema);
        let bogus = load_budget(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(first_reason(&bogus), Reason::Schema);
    }

    #[test]
    fn a_negative_budget_names_the_field() {
        let text = "schema: budget\npause:\n  named_plot_clocks: pause_when_empty\nmax_npcs_acting: -1\nidle_catchup_max_rounds: -3\n";
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
    fn docks_budget_leaves_idle_on() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/rules/budget.yaml");
        let text = std::fs::read_to_string(path).unwrap();
        let budget = load_budget(&text).unwrap();
        assert!(budget.idle_rounds);
        assert_eq!(budget.round_every_real_seconds, 300);
        assert_eq!(budget.idle_minutes_per_round, 60);
    }

    #[test]
    fn a_bad_idle_word_and_a_zero_interval_are_schema() {
        use crate::diagnostic::first_reason;
        let mut maybe = text("budget");
        maybe.push_str("idle: maybe\n");
        let err = load_budget(&maybe).unwrap_err();
        assert_eq!(first_reason(&err), Reason::Schema);
        assert_eq!(err[0].file, "rules/budget.yaml");
        assert!(err[0].line.is_some(), "{err:?}");
        assert!(err[0].message.contains("maybe"), "{err:?}");
        assert!(err[0].message.contains("on or off"), "{err:?}");

        let zero = "schema: budget\npause:\n  named_plot_clocks: pause_when_empty\nround_every_real_seconds: 0\n";
        let err = load_budget(zero).unwrap_err();
        assert_eq!(err[0].code, crate::diagnostic::Code::Error(Reason::Schema));
        assert!(err[0].line.is_some(), "{err:?}");
        assert!(
            err[0].message.contains("round_every_real_seconds"),
            "{err:?}"
        );
        assert!(err[0].message.contains('0'), "{err:?}");

        let minutes = "schema: budget\npause:\n  named_plot_clocks: pause_when_empty\nidle_minutes_per_round: 0\n";
        let err = load_budget(minutes).unwrap_err();
        assert_eq!(err[0].code, crate::diagnostic::Code::Error(Reason::Schema));
        assert!(err[0].line.is_some(), "{err:?}");
        assert!(err[0].message.contains("idle_minutes_per_round"), "{err:?}");

        let empty = "schema: budget\npause:\n  named_plot_clocks: pause_when_empty\nidle:\n";
        let err = load_budget(empty).unwrap_err();
        assert_eq!(first_reason(&err), Reason::Schema);
    }
}
