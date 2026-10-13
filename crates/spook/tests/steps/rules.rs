use std::fs;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::World;

use super::table;

#[given(expr = "a save with rules_rev 3 and an engine at 4")]
fn a_save_with_rules_rev_3_and_an_engine_at_4(world: &mut World) {
    assert_eq!(
        spookery_kernel::store::RULES_REV,
        4,
        "the engine rules version is 4"
    );
    let pack = crate::support::temp_pack("rules-rev");
    let path = pack.join("state/save.yaml");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read save: {err}"));
    let stamped = text.replacen("rules_rev: 4\n", "rules_rev: 3\n", 1);
    assert_ne!(stamped, text, "save did not carry rules_rev 4\n{text}");
    fs::write(&path, stamped).unwrap_or_else(|err| panic!("write save: {err}"));
    world.pack = Some(pack);
}

#[when(expr = "spook run or spook verify opens it")]
fn spook_run_or_spook_verify_opens_it(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    let arg = pack.to_str().unwrap();
    let port = table::free_port();
    let run = crate::support::spook(&["run", arg, "--addr", &format!("127.0.0.1:{port}")]);
    let verify = crate::support::spook(&["verify", arg]);
    let run_err = String::from_utf8_lossy(&run.stderr);
    let verify_err = String::from_utf8_lossy(&verify.stderr);
    world.code = if run.status.code() == Some(1) && verify.status.code() == Some(1) {
        Some(1)
    } else {
        None
    };
    world.stdout = format!(
        "run:\n{}\nverify:\n{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&verify.stdout)
    );
    world.stderr = format!("run:\n{run_err}\nverify:\n{verify_err}");
}

#[then(expr = "it refuses with RULES_REV and says to start a fresh save")]
fn it_refuses_with_rules_rev_and_says_to_start_a_fresh_save(world: &mut World) {
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(
        world.stderr.matches("RULES_REV").count() >= 2,
        "{}",
        world.stderr
    );
    assert!(
        world.stderr.matches("start a fresh save").count() >= 2,
        "{}",
        world.stderr
    );
}

#[given(expr = "logged effects that differ from the re-run")]
fn logged_effects_that_differ_from_the_re_run(world: &mut World) {
    let pack = crate::support::temp_pack("rules-diverge");
    world.pack = Some(pack.clone());
    table::run(world, &["tick", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(0), "{}", world.stderr);
    let path = pack.join("state/journal/tick-0001.jsonl");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read tick: {err}"));
    let mut tick: Value = serde_json::from_str(text.trim_end()).unwrap_or_else(|err| {
        panic!("json: {err}\n{text}");
    });
    let effects = tick["effects"]
        .as_array_mut()
        .unwrap_or_else(|| panic!("no effects\n{text}"));
    assert!(!effects.is_empty(), "{text}");
    effects[0]["intent_id"] = Value::String("not-the-replayed-effect".to_string());
    let index = tick["tick_index"]
        .as_u64()
        .unwrap_or_else(|| panic!("no tick_index\n{text}"));
    world.scratch = format!("tick {index}");
    let mut encoded = serde_json::to_string(&tick).unwrap_or_else(|err| panic!("encode: {err}"));
    encoded.push('\n');
    fs::write(&path, encoded).unwrap_or_else(|err| panic!("write tick: {err}"));
}

#[then(expr = "spook verify names the tick and the first differing effect")]
fn spook_verify_names_the_tick_and_the_first_differing_effect(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::run(world, &["verify", pack.to_str().unwrap()]);
    assert_eq!(world.code, Some(1), "{}", world.stderr);
    assert!(
        world.stderr.contains(world.scratch.trim()),
        "missing {}\n{}",
        world.scratch,
        world.stderr
    );
    assert!(
        world.stderr.contains("effect 0 differed"),
        "{}",
        world.stderr
    );
    assert!(world.stderr.contains("logged "), "{}", world.stderr);
    assert!(world.stderr.contains("replayed "), "{}", world.stderr);
}
