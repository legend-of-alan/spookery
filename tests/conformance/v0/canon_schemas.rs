use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

fn schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schema")
}

fn load_schema(name: &str) -> Value {
    let path = schema_dir().join(format!("{name}.schema.json"));
    let text =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{name} is not JSON: {err}"))
}

fn with_unknown(valid: &Value, path: &[&str]) -> Value {
    let mut doc = valid.clone();
    let mut cursor = &mut doc;
    for key in path {
        cursor = if let Ok(index) = key.parse::<usize>() {
            &mut cursor.as_array_mut().unwrap()[index]
        } else {
            cursor.get_mut(*key).unwrap()
        };
    }
    cursor
        .as_object_mut()
        .unwrap()
        .insert("not_in_spec".to_string(), json!(true));
    doc
}

fn assert_closed(name: &str, valid: Value, nested: &[&[&str]]) {
    let schema = load_schema(name);
    assert!(
        jsonschema::draft202012::meta::is_valid(&schema),
        "{name} schema does not load as draft 2020-12: {}",
        jsonschema::draft202012::meta::validate(&schema)
            .err()
            .map(|err| err.to_string())
            .unwrap_or_default()
    );
    let validator = jsonschema::draft202012::new(&schema)
        .unwrap_or_else(|err| panic!("{name} schema did not build: {err}"));
    assert!(
        validator.is_valid(&valid),
        "{name} rejected a canon object: {}",
        validator
            .validate(&valid)
            .err()
            .map(|err| err.to_string())
            .unwrap_or_default()
    );
    let mut paths: Vec<&[&str]> = vec![&[]];
    paths.extend(nested);
    for path in paths {
        let unknown = with_unknown(&valid, path);
        assert!(
            !validator.is_valid(&unknown),
            "{name} accepted an unknown key at {}",
            if path.is_empty() {
                "the object".to_string()
            } else {
                path.join(".")
            }
        );
    }
}

#[test]
fn v0_canon_schemas_reject_unknown_keys() {
    assert_closed(
        "place",
        json!({
            "schema": "place/v0",
            "id": "docks_shop",
            "facts": [{
                "id": "fact_oil_on_shelf",
                "text": "Oil is on the shelf.",
                "hidden": false
            }]
        }),
        &[&["facts", "0"]],
    );
    assert_closed(
        "cast",
        json!({
            "schema": "cast/v0",
            "id": "mira",
            "kind": "npc",
            "facts": [{
                "id": "fact_ledger_short",
                "text": "The lamp-oil ledger is short.",
                "hidden": true
            }],
            "tick": {
                "eligible": true,
                "tier": "named",
                "plot_relevant": false,
                "range": ["docks_shop", "docks_street", "mira_rooms"],
                "schedule": [
                    {"start": 6, "end": 18, "place": "docks_shop", "activity": "work"},
                    {"start": 18, "end": 6, "place": "mira_rooms"}
                ],
                "stock_rate": {"docks_shop": {"lamp_oil": 4}},
                "needs": {"rest": {"min": 0, "max": 5}},
                "resources": {},
                "flags": [],
                "work": {
                    "needs": [{"key": "rest", "delta": 1}],
                    "resources": []
                },
                "notes": [{
                    "id": "note_mira_door",
                    "due_clock": "shop_day",
                    "due_value": 1,
                    "to": ["pc_rowan"],
                    "place": "docks_shop",
                    "fact_ids": [],
                    "text": "Back at dusk. Oil on the shelf."
                }]
            }
        }),
        &[
            &["facts", "0"],
            &["tick"],
            &["tick", "schedule", "0"],
            &["tick", "needs", "rest"],
            &["tick", "work"],
            &["tick", "work", "needs", "0"],
            &["tick", "notes", "0"],
        ],
    );
    assert_closed(
        "moves",
        json!({
            "schema": "moves/v0",
            "moves": [
                {"name": "wait"},
                {"name": "attempt", "check": "grit"},
                {
                    "name": "mark",
                    "token": "actor",
                    "on": "named",
                    "effects": [{
                        "class": "flag",
                        "key": "marked",
                        "value": true
                    }]
                },
                {
                    "name": "wave",
                    "token": "none",
                    "on": "speaker",
                    "effects": [{
                        "class": "flag",
                        "key": "marked",
                        "value": false
                    }]
                }
            ]
        }),
        &[
            &["moves", "0"],
            &["moves", "2"],
            &["moves", "2", "effects", "0"],
            &["moves", "3"],
        ],
    );
    assert_closed(
        "checks",
        json!({
            "schema": "checks/v0",
            "checks": [{
                "id": "grit",
                "dice": {"count": 2, "sides": 6},
                "stat": "grit"
            }]
        }),
        &[&["checks", "0"], &["checks", "0", "dice"]],
    );
    assert_closed(
        "tick",
        json!({
            "schema": "tick/v0",
            "max_intents_applied": 12,
            "max_npcs_acting": 8,
            "llm_npc_intents_per_tick": 0,
            "max_effects_per_intent": 4,
            "max_state_keys_mutated": 24,
            "max_memory_writes": 8,
            "max_rumor_hops": 2,
            "idle_acts_per_world_day": 3,
            "live_acts_per_world_hour": 6,
            "tick_every_real_seconds": 300,
            "live_tick_every_real_seconds": 60,
            "idle_minutes_per_tick": 60,
            "live_minutes_per_tick": 5,
            "idle_catchup_max_ticks": 12,
            "idle_max_world_hours_per_real_hour": 8,
            "pause": {
                "named_plot_clocks": "pause_when_empty",
                "subsistence_clocks": "always_run",
                "relationship_drift": "always_run",
                "travel": "run_if_schedule"
            }
        }),
        &[&["pause"]],
    );
    assert_closed(
        "sheet",
        json!({
            "schema": "sheet/v0",
            "id": "pc_rowan",
            "template": "pc-harbormouse",
            "stats": {"grit": 1},
            "resources": {},
            "inventory": [],
            "flags": []
        }),
        &[],
    );
    assert_closed(
        "proposal",
        json!({
            "schema": "proposal/v0",
            "id": "season_advance",
            "kind": "other",
            "target": "",
            "summary": "season_advance",
            "actor_id": "pc_rowan",
            "source": "system",
            "tick_index": 12
        }),
        &[],
    );
}
