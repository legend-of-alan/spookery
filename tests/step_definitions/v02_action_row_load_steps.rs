use std::fs;

use cucumber::{then, when};

use spookery_kernel::canon::Canon;
use spookery_kernel::moves::{ActionRow, MoveOn, MoveToken, RowEffect};
use spookery_kernel::Reason;

use crate::support::temp_pack;
use crate::World;

const MARK_ROW: &str = "  - name: mark
    token: actor
    on: named
    effects:
      - class: flag
        key: marked
        value: true
";

const WAVE_ROW: &str = "  - name: wave
    token: none
    on: speaker
    effects:
      - class: flag
        key: marked
        value: false
";

fn append_row(text: &str, row: &str) -> String {
    let mut next = String::with_capacity(text.len() + row.len());
    next.push_str(text);
    next.push_str(row);
    next
}

fn rewrite(label: &str, text: &str) -> String {
    match label {
        "p06-location-key" => append_row(
            text,
            "  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: location\n        key: marked\n",
        ),
        "p06-location-value" => append_row(
            text,
            "  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: location\n        value: true\n",
        ),
        "p06-location-other" => append_row(
            text,
            "  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: location\n        delta: 1\n",
        ),
        "p06-place-flag" => append_row(
            text,
            "  - name: step\n    token: place\n    on: speaker\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        ),
        "p06-place-named" => append_row(
            text,
            "  - name: step\n    token: place\n    on: named\n    effects:\n      - class: location\n",
        ),
        "p06-actor-location" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: location\n",
        ),
        "p06-none-location" => append_row(
            text,
            "  - name: wave\n    token: none\n    on: speaker\n    effects:\n      - class: location\n",
        ),
        "p06-actor-speaker" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: speaker\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        ),
        "p06-none-named" => append_row(
            text,
            "  - name: mark\n    token: none\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        ),
        "p06-travel-flag" => text.replacen(
            "  - name: travel\n    token: place\n    on: speaker\n    effects:\n      - class: location\n",
            "  - name: travel\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
            1,
        ),
        "p06-attack" => text.replacen(
            "  - name: attack\n",
            "  - name: attack\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
            1,
        ),
        "p06-busy" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: busy\n        value: true\n",
        ),
        "p06-empty" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: named\n    effects: []\n",
        ),
        "p06-missing" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n",
        ),
        "p06-extra" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: named\n    effects:\n      - class: flag\n        key: marked\n        value: true\n        delta: 1\n",
        ),
        "p06-check" => append_row(
            text,
            "  - name: mark\n    token: actor\n    on: named\n    check: grit\n    effects:\n      - class: flag\n        key: marked\n        value: true\n",
        ),
        "p06-ok" => {
            let mut next = append_row(text, MARK_ROW);
            next.push_str(WAVE_ROW);
            next
        }
        other => panic!("unknown rewrite {other}"),
    }
}

fn load(w: &World) -> Canon {
    let label = w.scratch.get("label").map(String::as_str).unwrap_or("p06");
    spookery_kernel::canon::load_canon(w.pack.as_ref().unwrap())
        .unwrap_or_else(|err| panic!("{label} pack load: {err}"))
}

fn move_row<'a>(canon: &'a Canon, name: &str) -> &'a ActionRow {
    canon
        .moves
        .get(name)
        .unwrap_or_else(|| panic!("{name} missing"))
        .row
        .as_ref()
        .unwrap_or_else(|| panic!("{name} has no row"))
}

#[when(expr = "moves.yaml is rewritten as {string}")]
fn rewrite_moves(w: &mut World, label: String) {
    if let Some(old) = w.pack.take() {
        let _ = fs::remove_dir_all(old);
    }
    let pack = temp_pack(&label);
    let path = pack.join("rules/moves.yaml");
    let text = fs::read_to_string(&path).unwrap();
    let next = rewrite(&label, &text);
    assert_ne!(next, text, "{label} did not change moves.yaml");
    fs::write(&path, next).unwrap();
    w.pack = Some(pack);
    w.scratch.insert("label".to_string(), label);
}

#[then(expr = "pack load fails with SCHEMA")]
fn pack_load_fails_with_schema(w: &mut World) {
    let label = w.scratch.get("label").map(String::as_str).unwrap_or("p06");
    let err = spookery_kernel::canon::load_canon(w.pack.as_ref().unwrap()).expect_err("pack load");
    assert_eq!(err, Reason::Schema, "{label}");
}

#[then(expr = "move {string} check is {string}")]
fn move_check_is(w: &mut World, name: String, expected: String) {
    let canon = load(w);
    let check = canon
        .moves
        .get(&name)
        .unwrap_or_else(|| panic!("{name} missing"))
        .check
        .as_deref();
    assert_eq!(check, Some(expected.as_str()));
}

#[then(expr = "move {string} check is absent")]
fn move_check_absent(w: &mut World, name: String) {
    let canon = load(w);
    assert_eq!(
        canon
            .moves
            .get(&name)
            .unwrap_or_else(|| panic!("{name} missing"))
            .check
            .as_deref(),
        None
    );
}

#[then(expr = "move {string} token is {word}")]
fn move_token_is(w: &mut World, name: String, word: String) {
    let token = match word.as_str() {
        "none" => MoveToken::None,
        "actor" => MoveToken::Actor,
        "place" => MoveToken::Place,
        other => panic!("unknown token {other}"),
    };
    let canon = load(w);
    assert_eq!(move_row(&canon, &name).token, token);
}

#[then(expr = "move {string} on is {word}")]
fn move_on_is(w: &mut World, name: String, word: String) {
    let on = match word.as_str() {
        "speaker" => MoveOn::Speaker,
        "named" => MoveOn::Named,
        other => panic!("unknown on {other}"),
    };
    let canon = load(w);
    assert_eq!(move_row(&canon, &name).on, on);
}

#[then(expr = "move {string} effects are one flag {string} {word}")]
fn move_one_flag(w: &mut World, name: String, key: String, value: String) {
    let flag_value = match value.as_str() {
        "true" => true,
        "false" => false,
        other => panic!("unknown flag value {other}"),
    };
    let canon = load(w);
    assert_eq!(
        move_row(&canon, &name).effects,
        vec![RowEffect::Flag {
            key,
            value: flag_value,
        }]
    );
}

#[then(expr = "move {string} effects are location")]
fn move_location(w: &mut World, name: String) {
    let canon = load(w);
    assert_eq!(move_row(&canon, &name).effects, vec![RowEffect::Location]);
}

#[then(expr = "move {string} has no action row")]
fn move_has_no_row(w: &mut World, name: String) {
    let canon = load(w);
    assert!(canon
        .moves
        .get(&name)
        .unwrap_or_else(|| panic!("{name} missing"))
        .row
        .is_none());
}
