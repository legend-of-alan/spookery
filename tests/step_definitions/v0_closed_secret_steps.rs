use std::fs;
use std::path::Path;

use cucumber::{given, then};

use crate::World;

fn read_optional(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

fn perceive_debug(pack: &Path, as_arg: &str) -> String {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    let view =
        spookery_kernel::perceive(&shard, as_arg).unwrap_or_else(|err| panic!("perceive: {err}"));
    format!("{view:?}")
}

#[given(expr = "cast mira.md contains {string}")]
fn mira_file_contains(w: &mut World, sentence: String) {
    let pack = w.pack.as_ref().unwrap();
    let mira = fs::read_to_string(pack.join("cast/mira.md")).unwrap();
    assert!(mira.contains(&sentence), "fixture secret is on mira");
}

#[given(expr = "cast aldric.md does not contain {string}")]
fn aldric_file_lacks(w: &mut World, sentence: String) {
    let pack = w.pack.as_ref().unwrap();
    let aldric = fs::read_to_string(pack.join("cast/aldric.md")).unwrap();
    assert!(
        !aldric.contains(&sentence),
        "fixture secret is absent on aldric"
    );
}

#[then(expr = "Aldric perceive does not contain {string}")]
fn aldric_perceive_lacks(w: &mut World, sentence: String) {
    let pack = w.pack.as_ref().unwrap();
    let perceive = perceive_debug(pack, "npc:aldric");
    assert!(
        !perceive.contains(&sentence),
        "ledger sentence in Aldric's perceive"
    );
}

#[then(expr = "Aldric memories do not contain {string}")]
fn aldric_memories_lack(w: &mut World, sentence: String) {
    let pack = w.pack.as_ref().unwrap();
    let memories = read_optional(&pack.join("state/memories/aldric.jsonl"));
    assert!(
        !memories.contains(&sentence),
        "ledger sentence in Aldric's memories"
    );
}

#[then(expr = "rumors do not contain {string}")]
fn rumors_lack(w: &mut World, sentence: String) {
    let pack = w.pack.as_ref().unwrap();
    let rumors = format!(
        "{}\n{}",
        read_optional(&pack.join("state/rumors.yaml")),
        read_optional(&pack.join("state/now.yaml"))
    );
    assert!(!rumors.contains(&sentence), "ledger sentence in rumors");
}

#[then(expr = "Mira perceive contains fact_ledger_short text {string}")]
fn mira_perceives_ledger(w: &mut World, sentence: String) {
    let pack = w.pack.as_ref().unwrap();
    let perceive = perceive_debug(pack, "npc:mira");
    assert!(
        perceive.contains("fact_ledger_short"),
        "mira perceive missing fact_ledger_short\n{perceive}"
    );
    assert!(
        perceive.contains(&sentence),
        "mira perceive missing ledger text\n{perceive}"
    );
}
