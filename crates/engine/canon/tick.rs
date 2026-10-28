//! Round budgets and pause policy from `rules/tick.yaml`.

use serde::Deserialize;

use crate::canon::source::{load_headed, At, Problem, Source, TICK_SCHEMA};
use crate::diagnostic::Diagnostic;

const TICK_FILE: &str = "rules/tick.yaml";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub max_intents_applied: u64,
    pub max_npcs_acting: u64,
    pub max_effects_per_intent: u64,
    pub max_state_keys_mutated: u64,
    pub max_memory_writes: u64,
    pub idle_acts_per_world_day: u64,
    pub live_acts_per_world_hour: u64,
    pub tick_every_real_seconds: u64,
    /// Empty-table wakes. `false` leaves an empty table still.
    pub idle_ticking: bool,
    pub idle_minutes_per_tick: u64,
    pub live_minutes_per_tick: u64,
    pub idle_catchup_max_ticks: u64,
    pub idle_max_world_hours_per_real_hour: u64,
    pub max_fastforward_ticks: u64,
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
    PauseWhenEmpty,
    #[serde(rename = "always_run")]
    AlwaysRun,
}

impl Budget {
    pub(crate) fn defaults() -> Self {
        Self {
            max_intents_applied: 12,
            max_npcs_acting: 8,
            max_effects_per_intent: 4,
            max_state_keys_mutated: 24,
            max_memory_writes: 8,
            idle_acts_per_world_day: 3,
            live_acts_per_world_hour: 6,
            tick_every_real_seconds: 300,
            idle_ticking: true,
            idle_minutes_per_tick: 60,
            live_minutes_per_tick: 5,
            idle_catchup_max_ticks: 12,
            idle_max_world_hours_per_real_hour: 8,
            max_fastforward_ticks: 12,
            max_trigger_depth: 4,
            pause: PausePolicies {
                named_plot_clocks: PausePolicy::PauseWhenEmpty,
            },
        }
    }
}

pub(crate) fn load_tick(text: &str) -> Result<Budget, Vec<Diagnostic>> {
    let file: TickFile = load_headed(TICK_FILE, text, TICK_SCHEMA, 0)?;
    let source = Source {
        file: TICK_FILE,
        text,
        line_base: 0,
    };
    let mut diags = Vec::new();
    let mut budget = Budget::defaults();
    // Each row is a counter key and the least value it allows, in report order.
    macro_rules! counters {
        ($($key:ident >= $least:literal),* $(,)?) => {$(
            take(&source, stringify!($key), file.$key, &mut budget.$key, $least, &mut diags);
        )*};
    }
    counters!(
        max_intents_applied >= 0,
        max_npcs_acting >= 0,
        max_effects_per_intent >= 0,
        max_state_keys_mutated >= 0,
        max_memory_writes >= 0,
        idle_acts_per_world_day >= 0,
        live_acts_per_world_hour >= 0,
        tick_every_real_seconds >= 1,
    );
    budget.idle_ticking = take_idle(&source, file.idle, &mut diags);
    counters!(
        idle_minutes_per_tick >= 1,
        live_minutes_per_tick >= 0,
        idle_catchup_max_ticks >= 0,
        idle_max_world_hours_per_real_hour >= 0,
        max_fastforward_ticks >= 0,
        max_trigger_depth >= 0,
    );
    budget.pause.named_plot_clocks = file.pause.named_plot_clocks;
    if !diags.is_empty() {
        return Err(diags);
    }
    Ok(budget)
}

/// `value` into `slot` when it is at least `least`; otherwise `slot` keeps its default.
fn take(
    source: &Source,
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
            at: At::Key(&format!("{key}:")),
            problem,
            hint,
        }
        .schema(),
    );
}

fn take_idle(source: &Source, value: Option<String>, diags: &mut Vec<Diagnostic>) -> bool {
    match value.as_deref() {
        None | Some("on") => true,
        Some("off") => false,
        Some(other) => {
            diags.push(
                Problem {
                    source,
                    path: "idle",
                    at: At::Key("idle:"),
                    problem: format!("idle is '{other}', which is not on or off"),
                    hint: "write idle: on or idle: off",
                }
                .schema(),
            );
            true
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TickFile {
    #[serde(rename = "schema")]
    _schema: String,
    max_intents_applied: Option<i64>,
    max_npcs_acting: Option<i64>,
    max_effects_per_intent: Option<i64>,
    max_state_keys_mutated: Option<i64>,
    max_memory_writes: Option<i64>,
    idle_acts_per_world_day: Option<i64>,
    live_acts_per_world_hour: Option<i64>,
    tick_every_real_seconds: Option<i64>,
    #[serde(default, deserialize_with = "read_idle_word")]
    idle: Option<String>,
    idle_minutes_per_tick: Option<i64>,
    live_minutes_per_tick: Option<i64>,
    idle_catchup_max_ticks: Option<i64>,
    idle_max_world_hours_per_real_hour: Option<i64>,
    max_fastforward_ticks: Option<i64>,
    max_trigger_depth: Option<i64>,
    pause: PauseFile,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PauseFile {
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
    use super::load_tick;
    use crate::error::Reason;

    fn text(tag: &str) -> String {
        format!("schema: {tag}\npause:\n  named_plot_clocks: pause_when_empty\n")
    }

    #[test]
    fn an_unknown_tag_and_the_old_versioned_tag_are_schema() {
        use crate::diagnostic::reason_of;
        assert!(load_tick(&text("tick")).is_ok());
        let old = load_tick(&text(concat!("tick/", "v0"))).unwrap_err();
        assert!(old[0].message.contains("no version"), "{old:?}");
        assert_eq!(reason_of(old), Reason::Schema);
        let bogus = load_tick(&text("bogus")).unwrap_err();
        assert!(bogus[0].message.contains("does not use"), "{bogus:?}");
        assert_eq!(reason_of(bogus), Reason::Schema);
    }

    #[test]
    fn a_negative_budget_names_the_field() {
        let text = "schema: tick\npause:\n  named_plot_clocks: pause_when_empty\nmax_npcs_acting: -1\nidle_catchup_max_ticks: -3\n";
        let err = load_tick(text).unwrap_err();
        assert!(err
            .iter()
            .any(|diag| diag.message.contains("max_npcs_acting") && diag.line.is_some()));
        assert!(err
            .iter()
            .any(|diag| diag.message.contains("idle_catchup_max_ticks")));
    }

    #[test]
    fn the_removed_live_tick_key_is_refused() {
        use crate::diagnostic::reason_of;
        let mut stale = text("tick");
        stale.push_str("live_tick_every_real_seconds: 60\n");
        let err = load_tick(&stale).unwrap_err();
        assert_eq!(reason_of(err), Reason::Schema);
    }

    #[test]
    fn idle_defaults_to_on_and_off_is_loaded() {
        let on = load_tick(&text("tick")).unwrap();
        assert!(on.idle_ticking);
        let mut explicit = text("tick");
        explicit.push_str("idle: on\n");
        assert!(load_tick(&explicit).unwrap().idle_ticking);
        let mut off = text("tick");
        off.push_str("idle: off\n");
        assert!(!load_tick(&off).unwrap().idle_ticking);
    }

    #[test]
    fn docks_tick_rules_leave_idle_on() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/rules/tick.yaml");
        let text = std::fs::read_to_string(path).unwrap();
        let budget = load_tick(&text).unwrap();
        assert!(budget.idle_ticking);
        assert_eq!(budget.tick_every_real_seconds, 300);
        assert_eq!(budget.idle_minutes_per_tick, 60);
    }

    #[test]
    fn a_bad_idle_word_and_a_zero_interval_are_schema() {
        use crate::diagnostic::reason_of;
        let mut maybe = text("tick");
        maybe.push_str("idle: maybe\n");
        let err = load_tick(&maybe).unwrap_err();
        assert_eq!(reason_of(err.clone()), Reason::Schema);
        assert_eq!(err[0].file, "rules/tick.yaml");
        assert!(err[0].line.is_some(), "{err:?}");
        assert!(err[0].message.contains("maybe"), "{err:?}");
        assert!(err[0].message.contains("on or off"), "{err:?}");

        let zero = "schema: tick\npause:\n  named_plot_clocks: pause_when_empty\ntick_every_real_seconds: 0\n";
        let err = load_tick(zero).unwrap_err();
        assert_eq!(
            err[0].code,
            crate::diagnostic::Severity::Error(Reason::Schema)
        );
        assert!(err[0].line.is_some(), "{err:?}");
        assert!(
            err[0].message.contains("tick_every_real_seconds"),
            "{err:?}"
        );
        assert!(err[0].message.contains('0'), "{err:?}");

        let minutes = "schema: tick\npause:\n  named_plot_clocks: pause_when_empty\nidle_minutes_per_tick: 0\n";
        let err = load_tick(minutes).unwrap_err();
        assert_eq!(
            err[0].code,
            crate::diagnostic::Severity::Error(Reason::Schema)
        );
        assert!(err[0].line.is_some(), "{err:?}");
        assert!(err[0].message.contains("idle_minutes_per_tick"), "{err:?}");

        let empty = "schema: tick\npause:\n  named_plot_clocks: pause_when_empty\nidle:\n";
        let err = load_tick(empty).unwrap_err();
        assert_eq!(reason_of(err), Reason::Schema);
    }
}
