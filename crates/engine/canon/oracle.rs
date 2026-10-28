//! A temporary byte-for-byte oracle for the canon refactor (`docs/refactor/canon-plan.md`).
//!
//! `SPOOK_ORACLE=write` loads every case below and writes `canon.txt`, `load.json`,
//! `check.json` and `rev.txt` under `docs/refactor/oracle/<case>/`. `SPOOK_ORACLE=check`
//! rebuilds them and compares bytes. Without the variable the test does nothing, so CI is
//! unchanged. The baselines are never committed. Packs are copied into temp dirs.
//!
//! The three "could not read world/, cast/, story/" sites need a directory the process
//! cannot read, which cannot be set up portably, so they have no case and no `EXPECTED` entry.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::{load_canon, pack_rev, Canon, CastCanon, Exit, FactCanon, ItemCanon, PlaceCanon};
use crate::testing::{copy_pack, fresh_dir};
use crate::{check_pack, Diagnostic};

use Base::{Docks, Empty, Reference, D20};
use Edit::{Add, Crlf, Del, Raw, Sub};

#[derive(Clone, Copy)]
enum Base {
    Docks,
    D20,
    Reference,
    Empty,
}

/// One change to a copied pack, applied in order. `Sub` replaces the first match.
enum Edit {
    Sub(&'static str, &'static str, &'static str),
    Add(&'static str, &'static str),
    Raw(&'static str, &'static [u8]),
    Del(&'static str),
    /// Rewrite `pack.yaml` and every file under `world/`, `cast/` and `rules/` with CRLF.
    Crlf,
}

const PACK: &str = "pack.yaml";
const SHOP: &str = "world/places/docks_shop.md";
const STREET: &str = "world/places/docks_street.md";
const ROOMS: &str = "world/places/mira_rooms.md";
const KEY: &str = "world/items/brass_key.md";
const OIL: &str = "world/items/lamp_oil.md";
const MIRA: &str = "cast/mira.md";
const BRAM: &str = "cast/bram.md";
const VERBS: &str = "rules/verbs.yaml";
const CHECKS: &str = "rules/checks.yaml";
const METERS: &str = "rules/meters.yaml";
const TICK: &str = "rules/tick.yaml";
const SAVE: &str = "state/save.yaml";
const STORY: &str = "story/s.yaml";
const UNWIELD: &str = "      - unwield:\n          who: self";
const GRIT_ROLL: &str = "      target: 0\n    do: []";
const ONE_UNWIELD: &str = "\n      - unwield:\n          who: self";

/// A story file with one change rule `r`: the watch body, then the rest of the rule.
macro_rules! story {
    ($watch:literal, $rest:literal) => {
        concat!(
            "schema: story\nid: s\nrules:\n  - id: r\n    when:\n      change:\n",
            $watch,
            $rest
        )
    };
}

#[rustfmt::skip]
const CASES: &[(&str, Base, &[Edit])] = &[
    // Valid packs.
    ("valid_docks", Docks, &[]),
    ("valid_d20", D20, &[]),
    ("valid_reference", Reference, &[]),
    ("valid_minimal", Empty, &[
        Add(PACK, "schema: pack\nid: ex\ntitle: Ex\nsystem: ex\nstart:\n  place: gate\n  pc: pc_rowan\n"),
        Add("world/places/gate.md", "---\nschema: place\nid: gate\ndisplay_name: Gate\nfacts:\n  - id: fact_gate\n    text: The gate is shut.\n    hidden: false\n---\nA gate.\n"),
        Add("cast/pc_rowan.md", "---\nschema: cast\nid: pc_rowan\nkind: pc_template\ndisplay_name: Rowan\nfacts: []\n---\nRowan.\n"),
        Add(VERBS, include_str!("../../../packs/oneshot-docks/rules/verbs.yaml")),
        Add(CHECKS, include_str!("../../../packs/oneshot-docks/rules/checks.yaml")),
        Add(METERS, include_str!("../../../packs/oneshot-docks/rules/meters.yaml")),
        Add(TICK, include_str!("../../../packs/oneshot-docks/rules/tick.yaml")),
    ]),
    ("valid_docks_nell", Docks, &[Add("cast/pc-nell.md", include_str!("../fixtures/cast/pc-nell.md"))]),
    ("valid_docks_crlf", Docks, &[Crlf]),
    ("valid_docks_fence_at_end", Docks, &[Sub(STREET, "\n---\nThe street runs between the shop and the rooms.\n", "\n---")]),
    ("valid_docks_crlf_fence_at_end", Docks, &[Crlf, Sub(STREET, "\r\n---\r\nThe street runs between the shop and the rooms.\r\n", "\r\n---")]),
    ("valid_skipped_files", Docks, &[Add("world/notes.txt", "junk"), Add("world/lore/old.md", "---\nfoo: 1\n---\n"), Add("cast/notes.txt", "junk"), Add("story/readme.md", "junk")]),
    ("valid_null_cost", Docks, &[Sub(SHOP, "    cost:\n      time: 2m\n", "    cost: ~\n"), Sub(VERBS, "      verb: wait\n", "      verb: wait\n    cost: ~\n")]),
    // canon/mod.rs and the shared loader in diagnostic.rs.
    ("place_not_utf8", Docks, &[Raw(SHOP, b"---\n\xff\n---\n")]),
    ("cast_not_utf8", Docks, &[Raw(MIRA, b"---\n\xff\n---\n")]),
    ("story_not_utf8", Docks, &[Raw("story/bad.yaml", b"schema: story\n\xff\n")]),
    ("rules_not_utf8", Docks, &[Raw(CHECKS, b"schema: checks\n\xff\n")]),
    ("no_frontmatter", Docks, &[Sub(KEY, "---\nschema: item", "schema: item")]),
    ("no_closing_fence", Docks, &[Sub(BRAM, "\n---\nBram", "\nBram")]),
    ("item_id_empty", Docks, &[Sub(KEY, "id: brass_key", "id: \"\"")]),
    ("item_twice", Docks, &[Sub(OIL, "id: lamp_oil", "id: brass_key")]),
    ("fact_id_empty", Docks, &[Sub(STREET, "id: fact_street_quiet", "id: \"\"")]),
    // Decision D3 freezes the fact row one early: facts[1] is reported at facts[0]'s line.
    ("fact_twice_second_row", Docks, &[Sub(SHOP, "    hidden: false\nexits:", "    hidden: false\n  - id: fact_oil_on_shelf\n    text: Again.\n    hidden: false\nexits:")]),
    ("fact_twice_place", Docks, &[Sub(STREET, "id: fact_street_quiet", "id: fact_oil_on_shelf")]),
    ("fact_twice_cast", Docks, &[Sub(MIRA, "id: fact_ledger_short", "id: fact_room_lamp")]),
    ("place_twice", Docks, &[Sub(ROOMS, "id: mira_rooms", "id: docks_street")]),
    ("cast_twice", Docks, &[Sub(BRAM, "id: bram", "id: aldric")]),
    ("moves_present", Docks, &[Add("rules/moves.yaml", "schema: moves\n")]),
    ("pack_missing", Docks, &[Del(PACK)]),
    ("verbs_missing", Docks, &[Del(VERBS)]),
    ("checks_missing", Docks, &[Del(CHECKS)]),
    ("meters_missing", Docks, &[Del(METERS)]),
    ("tick_missing", Docks, &[Del(TICK)]),
    ("rule_twice_across_files", Docks, &[
        Add("story/a.yaml", "schema: story\nid: s\nrules:\n  - id: r\n    when:\n      verb: ra\n    do: []\n"),
        Add("story/b.yaml", "schema: story\nid: s\nrules:\n  - id: r\n    when:\n      verb: rb\n    do: []\n"),
    ]),
    ("checkpoint_zero", Docks, &[Sub(PACK, "start:", "checkpoint_every: 0\nstart:")]),
    ("exit_no_name", Docks, &[Sub(SHOP, "name: docks_street", "name: \"\"")]),
    ("exit_twice", Docks, &[Sub(SHOP, "    name: docks_street\n", "    name: stairs\n")]),
    ("exit_cost_fault", Docks, &[Sub(SHOP, "time: 2m", "time: soon")]),
    ("exit_cond_fault", Docks, &[Sub(SHOP, "who: self\n          id: brass_key", "who: rate\n          id: brass_key")]),
    ("meter_id_empty", Docks, &[Sub(METERS, "id: rest", "id: \"\"")]),
    ("meter_min_above_cap", Docks, &[Sub(METERS, "    min: 0\n    base_cap: 5", "    min: 6\n    base_cap: 5")]),
    ("meter_cap_above_max", Docks, &[Sub(METERS, "base_cap: 5\n    max: 5", "base_cap: 5\n    max: 4")]),
    ("meter_start_outside", Docks, &[Sub(METERS, "start: 3", "start: 9")]),
    ("meter_twice", Docks, &[Sub(METERS, "id: rest", "id: grit")]),
    ("check_id_empty", Docks, &[Sub(CHECKS, "id: grit", "id: \"\"")]),
    ("check_zero_dice", Docks, &[Sub(CHECKS, "count: 2", "count: 0")]),
    ("check_zero_sides", Docks, &[Sub(CHECKS, "sides: 6", "sides: 0")]),
    ("check_no_stat", Docks, &[Sub(CHECKS, "stat: grit", "stat: \"\"")]),
    ("check_twice", Docks, &[Sub(CHECKS, "    stat: grit\n", "    stat: grit\n  - id: grit\n    dice:\n      count: 1\n      sides: 6\n    stat: grit\n")]),
    ("check_negative_count", Docks, &[Sub(CHECKS, "count: 2", "count: -2")]),
    ("pack_id_empty", Docks, &[Sub(PACK, "id: docks-001", "id: \"\"")]),
    ("place_id_empty", Docks, &[Sub(STREET, "id: docks_street", "id: \"\"")]),
    ("place_name_empty", Docks, &[Sub(STREET, "display_name: Docks Street", "display_name: \" \"")]),
    ("cast_id_empty", Docks, &[Sub(BRAM, "id: bram", "id: \"\"")]),
    ("cast_id_world", Docks, &[Sub(BRAM, "id: bram", "id: world")]),
    ("cast_name_empty", Docks, &[Sub(BRAM, "display_name: Bram", "display_name: \"\"")]),
    ("cast_kind_ghost", Docks, &[Sub(BRAM, "kind: npc", "kind: ghost")]),
    ("tag_missing", Docks, &[Sub(KEY, "schema: item\n", "")]),
    ("tag_old", Docks, &[Sub(VERBS, "schema: verbs", "schema: verbs/v0")]),
    ("tag_wrong_yaml", Docks, &[Sub(METERS, "schema: meters", "schema: checks")]),
    ("tag_wrong_markdown", Docks, &[Sub(MIRA, "schema: cast", "schema: place")]),
    ("serde_unknown_field", Docks, &[Sub(TICK, "pause:", "paws: 1\npause:")]),
    ("serde_missing_field", Docks, &[Sub(PACK, "system: oneshot-docks\n", "")]),
    ("serde_wrong_type", Docks, &[Sub(PACK, "title: Oneshot Docks", "title: [a]")]),
    ("serde_invalid_type", Docks, &[Sub(MIRA, "    if:\n      - hour:\n          from: 8\n          to: 16\n", "    if: 3\n")]),
    ("serde_bad_scalar", Docks, &[Sub(METERS, "max: 110", "max: lots")]),
    ("serde_unknown_variant", Docks, &[Sub(TICK, "named_plot_clocks: pause_when_empty", "named_plot_clocks: sometimes")]),
    ("serde_null_string", Docks, &[Sub(PACK, "title: Oneshot Docks", "title: ~")]),
    ("serde_generic", Docks, &[Sub(SHOP, "time: 2m", "meter: 3")]),
    ("serde_bool_word", Docks, &[Add(STORY, story!("        flag: y\n        becomes: true\n", "    do: []\n"))]),
    // canon/schema.rs.
    ("rule_id_say", Docks, &[Sub(VERBS, "id: wait", "id: say")]),
    ("when_not_habit", Docks, &[Sub(MIRA, "    when: habit\n    if:\n      - hour:", "    when: often\n    if:\n      - hour:")]),
    ("verb_say", Docks, &[Sub(VERBS, "verb: wait", "verb: say")]),
    ("target_not_kind", Docks, &[Sub(VERBS, "target: exit", "target: door")]),
    ("when_no_verb_or_change", Docks, &[Sub(VERBS, "    when:\n      verb: wait\n", "    when:\n      target: none\n")]),
    ("change_meter_no_crosses", Docks, &[Add(STORY, story!("        meter: grit\n", "    do: []\n"))]),
    ("change_crosses_malformed", Docks, &[Add(STORY, story!("        meter: grit\n        crosses: about 3\n", "    do: []\n"))]),
    ("change_flag_no_becomes", Docks, &[Add(STORY, story!("        flag: lit\n", "    do: []\n"))]),
    ("change_item_no_gained", Docks, &[Add(STORY, story!("        item: brass_key\n", "    do: []\n"))]),
    ("change_no_watch", Docks, &[Add(STORY, story!("        who: pc\n", "    do: []\n"))]),
    ("cond_two_under_dash", Docks, &[Sub(MIRA, "          to: 16\n", "          to: 16\n        note_exists: x\n")]),
    ("cond_hour_range", Docks, &[Sub(MIRA, "to: 16", "to: 24")]),
    ("cond_op_bad", Docks, &[Sub(MIRA, "op: \">=\"", "op: \"=>\"")]),
    ("cond_dice", Docks, &[Sub(MIRA, "          value: 1\n", "          value: \"1d6\"\n")]),
    ("who_retired", Docks, &[Sub(VERBS, "who: here\n          id: target\n          add: -1", "who: token\n          id: target\n          add: -1")]),
    ("who_subject", Docks, &[Sub(VERBS, "who: self\n          id: target\n          add: 1", "who: subject\n          id: target\n          add: 1")]),
    ("who_place_no_id", Docks, &[Sub(MIRA, "who: place:docks_shop", "who: \"place:\"")]),
    ("expr_not_number", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"2 +\"\n")]),
    ("expr_no_dice", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"0d6\"\n")]),
    ("expr_many_dice", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"1001d6\"\n")]),
    ("expr_div_zero", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"4 / 0\"\n")]),
    ("time_not_duration", Docks, &[Sub(VERBS, "      verb: wait\n", "      verb: wait\n    cost:\n      time: later\n")]),
    ("time_bad_unit", Docks, &[Sub(VERBS, "      verb: wait\n", "      verb: wait\n    cost:\n      time: 2w\n")]),
    ("roll_check_empty", Docks, &[Sub(VERBS, "check: grit", "check: \"\"")]),
    ("change_has_cost", Docks, &[Add(STORY, story!("        clock: c\n", "    cost:\n      time: 1m\n    do: []\n"))]),
    ("outcome_no_roll", Docks, &[Sub(VERBS, "      verb: wait\n    do: []", "      verb: wait\n    do: []\n    success:\n      - unwield:\n          who: self")]),
    ("roll_no_success", Docks, &[Sub(VERBS, GRIT_ROLL, "      target: 0\n    do: []\n    failure:\n      - unwield:\n          who: self")]),
    ("partial_at_no_partial", Docks, &[Sub(VERBS, "      target: 0\n", "      target: 5\n      partial_at: 3\n")]),
    ("partial_no_partial_at", Docks, &[Sub(VERBS, GRIT_ROLL, concat!("      target: 0\n    do: []\n    success:", "\n      - unwield:\n          who: self", "\n    partial:", "\n      - unwield:\n          who: self"))]),
    ("partial_at_not_below", Docks, &[Sub(VERBS, GRIT_ROLL, concat!("      target: 3\n      partial_at: 3\n    do: []\n    success:", "\n      - unwield:\n          who: self", "\n    partial:", "\n      - unwield:\n          who: self"))]),
    ("meter_add_and_set", Docks, &[Sub(VERBS, UNWIELD, "      - meter:\n          who: self\n          key: grit\n          add: 1\n          set: 2")]),
    ("flag_busy", Docks, &[Sub(VERBS, UNWIELD, "      - flag:\n          who: self\n          key: busy\n          value: true")]),
    ("location_no_to", Docks, &[Sub(VERBS, "          via: target", "          teleport: true")]),
    ("note_incomplete", Docks, &[Sub(VERBS, UNWIELD, "      - note:\n          id: n\n          text: \"\"")]),
    ("clock_no_add", Docks, &[Sub(VERBS, UNWIELD, "      - clock:\n          id: c")]),
    ("effect_two_under_dash", Docks, &[Sub(VERBS, UNWIELD, "      - unwield:\n          who: self\n        clock:\n          id: c\n          start: true")]),
    ("resist_not_same", Docks, &[Sub(VERBS, "          id: target\n          add: 1\n", "          id: target\n          add: 1\n          resist: maybe\n")]),
    ("verb_twice", Docks, &[Sub(VERBS, "verb: drop", "verb: take")]),
    ("rule_twice_in_file", Docks, &[Sub(VERBS, "id: drop", "id: take")]),
    // canon/check.rs.
    ("exit_to_unknown", Docks, &[Sub(SHOP, "  - to: docks_street\n", "  - to: nowhere\n")]),
    ("exit_hidden_no_fact", Docks, &[Sub(STREET, "    name: docks_shop\n", "    name: docks_shop\n    hidden: true\n")]),
    ("exit_reveals_unknown", Docks, &[Sub(STREET, "    name: docks_shop\n", "    name: docks_shop\n    revealed_by: fact_nope\n")]),
    ("rule_reveals_unknown", Docks, &[Sub(VERBS, UNWIELD, "      - reveal:\n          who: self\n          fact: fact_nope")]),
    ("exit_unknown_item", Docks, &[Sub(SHOP, "id: brass_key", "id: iron_key")]),
    ("rule_unknown_item", Docks, &[Sub(MIRA, "id: lamp_oil", "id: whale_oil")]),
    ("resist_same_first", Docks, &[Sub(VERBS, "          id: target\n          add: 1\n", "          id: target\n          add: 1\n          resist: same\n")]),
    ("reveal_to_world", Docks, &[Sub(VERBS, UNWIELD, "      - reveal:\n          who: world\n          fact: fact_oil_on_shelf")]),
    ("unknown_check", Docks, &[Sub(VERBS, "check: grit", "check: courage")]),
    ("expr_reads_world", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"world.grit\"\n")]),
    ("expr_unknown_meter", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"self.luck\"\n")]),
    ("expr_unknown_who", Docks, &[Sub(VERBS, "      target: 0\n", "      target: \"nobody.grit\"\n")]),
    ("exit_expr_unknown_meter", Docks, &[Sub(SHOP, "      time: 2m\n", "      time: 2m\n      meter:\n        grit: \"self.zeal\"\n")]),
    ("check_stat_unknown", Docks, &[Sub(CHECKS, "stat: grit", "stat: luck")]),
    ("rule_writes_unknown_meter", Docks, &[Sub(VERBS, UNWIELD, "      - meter:\n          who: self\n          key: luck\n          add: 1")]),
    ("warn_no_path", Docks, &[Add("world/places/attic.md", "---\nschema: place\nid: attic\ndisplay_name: Attic\nfacts: []\n---\nDust.\n")]),
    ("warn_teleport", Docks, &[Sub(VERBS, "          via: target", "          to: docks_street\n          teleport: true")]),
    ("warn_trigger_cycle", Docks, &[Add(STORY, story!("        flag: lit\n        becomes: true\n", "    do:\n      - flag:\n          who: self\n          key: dim\n          value: true\n  - id: q\n    when:\n      change:\n        flag: dim\n        becomes: true\n    do:\n      - flag:\n          who: self\n          key: lit\n          value: true\n"))]),
    ("trigger_leak", Docks, &[Add(STORY, story!("        knows: fact_oil_on_shelf\n", "    do:\n      - narrate:\n          to: here\n          text: Someone learned it.\n"))]),
    // canon/round_settings.rs.
    ("tick_below_zero", Docks, &[Sub(TICK, "max_npcs_acting: 8", "max_npcs_acting: -1"), Sub(TICK, "tick_every_real_seconds: 300", "tick_every_real_seconds: -5")]),
    ("tick_interval_zero", Docks, &[Sub(TICK, "idle_minutes_per_tick: 60", "idle_minutes_per_tick: 0")]),
    ("tick_idle_maybe", Docks, &[Sub(TICK, "schema: tick\n", "schema: tick\nidle: maybe\n")]),
    ("tick_idle_empty", Docks, &[Sub(TICK, "schema: tick\n", "schema: tick\nidle:\n")]),
    // canon/save.rs through check_pack.
    ("save_lacks_meter", Docks, &[Sub(SAVE, "      rest:\n        base: 3\n        offset: 0\n", "")]),
    ("save_unknown_meter", Docks, &[Sub(SAVE, "      rest:\n        base: 3\n        offset: 0\n", "      rest:\n        base: 3\n        offset: 0\n      zeal:\n        base: 1\n        offset: 0\n")]),
    ("save_meter_outside", Docks, &[Sub(SAVE, "        base: 3\n", "        base: 99\n")]),
    ("save_empty_item_id", Docks, &[Sub(SAVE, "    inventory: {}\n", "    inventory:\n      \"\": 1\n")]),
    ("save_actor_zero_count", Docks, &[Sub(SAVE, "    inventory: {}\n", "    inventory:\n      lamp_oil: 0\n")]),
    ("save_place_zero_count", Docks, &[Sub(SAVE, "      lamp_oil: 4", "      lamp_oil: 0")]),
    ("save_unknown_inventory_item", Docks, &[Sub(SAVE, "    inventory: {}\n", "    inventory:\n      whale_oil: 1\n")]),
    ("save_unknown_wielded", Docks, &[Sub(SAVE, "    inventory: {}\n", "    inventory: {}\n    wielded: sword\n")]),
    ("save_unknown_place_item", Docks, &[Sub(SAVE, "      lamp_oil: 4", "      lamp_oil: 4\n      whale_oil: 1")]),
    ("save_wields_non_weapon", Docks, &[Sub(SAVE, "    inventory: {}\n", "    inventory:\n      lamp_oil: 1\n    wielded: lamp_oil\n")]),
    ("save_wields_unheld", Docks, &[Sub(KEY, "tags: []", "tags:\n  - weapon"), Sub(SAVE, "    inventory: {}\n", "    inventory: {}\n    wielded: brass_key\n")]),
    // Stop points of section 2.4.
    ("stop_place_bad_fact", Docks, &[Sub(SHOP, "id: fact_oil_on_shelf", "id: \"\""), Sub(SHOP, "name: docks_street", "name: \"\""), Sub(SHOP, "    locked_text:", "    hidden: true\n    locked_text:"), Sub(SHOP, "\n---\nThe shop", "\nrules:\n  - id: say\n    when: habit\n    do: []\n---\nThe shop")]),
    ("stop_place_exits_before_twice", Docks, &[Sub(ROOMS, "id: mira_rooms", "id: docks_street"), Sub(ROOMS, "name: stairs", "name: \"\""), Sub(ROOMS, "    hidden: false\n---", "    hidden: false\nrules:\n  - id: say\n    when: habit\n    do: []\n---")]),
    ("stop_place_head", Docks, &[Sub(STREET, "id: docks_street", "id: \"\""), Sub(STREET, "display_name: Docks Street", "display_name: \"\""), Sub(STREET, "id: fact_street_quiet", "id: \"\"")]),
    ("stop_item_rules_then_insert", Docks, &[Sub(KEY, "rules: []", "rules:\n  - id: say\n    when: habit\n    do: []")]),
    ("stop_item_id_skips_rules", Docks, &[Sub(KEY, "id: brass_key", "id: \"\""), Sub(KEY, "rules: []", "rules:\n  - id: say\n    when: habit\n    do: []")]),
    ("stop_item_twice_skips_rules", Docks, &[Sub(OIL, "id: lamp_oil", "id: brass_key"), Sub(OIL, "rules: []", "rules:\n  - id: say\n    when: habit\n    do: []")]),
    ("stop_cast_bad_fact", Docks, &[Sub(MIRA, "id: fact_ledger_short", "id: \"\""), Sub(MIRA, "id: mira", "id: aldric"), Sub(MIRA, "    when: habit\n    if:\n      - hour:", "    when: often\n    if:\n      - hour:")]),
    ("stop_cast_head", Docks, &[Sub(BRAM, "id: bram", "id: world"), Sub(BRAM, "display_name: Bram", "display_name: \"\""), Sub(BRAM, "kind: npc", "kind: ghost"), Sub(BRAM, "facts: []", "facts:\n  - id: \"\"\n    text: x\n    hidden: false")]),
    ("stop_rule_effect_drops_rest", Docks, &[Sub(VERBS, "id: unwield", "id: say"), Sub(VERBS, ONE_UNWIELD, "\n      - clock:\n          id: c")]),
    ("stop_exit_cost_skips_if", Docks, &[Sub(SHOP, "time: 2m", "time: soon"), Sub(SHOP, "who: self", "who: rate")]),
    ("stop_exit_conds_all", Docks, &[Sub(SHOP, "    if:\n      - has_item:", "    if:\n      - hour:\n          from: 1\n          to: 1\n      - has_item:"), Sub(SHOP, "who: self", "who: rate")]),
    ("stop_meter_bad_skips_twice", Docks, &[Sub(METERS, "id: rest\n    min: 0", "id: grit\n    min: 9")]),
    ("stop_check_bad_skips_twice", Docks, &[Sub(CHECKS, "    stat: grit\n", "    stat: grit\n  - id: grit\n    dice:\n      count: 0\n      sides: 6\n    stat: grit\n")]),
    ("stop_tag_before_shape", Docks, &[Sub(TICK, "schema: tick", "schema: tock\npaws: 1")]),
    ("stop_check_pack_needs_canon", Docks, &[Sub(BRAM, "kind: npc", "kind: ghost"), Sub(SAVE, "      lamp_oil: 4", "      lamp_oil: 0")]),
    // Order: two faults in one file, and faults in several files.
    ("order_one_file", Docks, &[Sub(VERBS, "id: wait", "id: say"), Sub(VERBS, "target: exit", "target: door"), Sub(VERBS, "check: grit", "check: \"\"")]),
    ("order_one_file_meters", Docks, &[Sub(METERS, "id: grit\n    min: 0\n    base_cap: 100", "id: grit\n    min: 101\n    base_cap: 100"), Sub(METERS, "base_cap: 5\n    max: 5", "base_cap: 5\n    max: 4")]),
    ("order_three_files", Docks, &[Sub(PACK, "start:", "checkpoint_every: 0\nstart:"), Sub(SHOP, "    name: docks_street\n", "    name: stairs\n"), Sub(MIRA, "kind: npc", "kind: ghost"), Sub(CHECKS, "count: 2", "count: 0"), Sub(TICK, "schema: tick\n", "schema: tick\nidle: maybe\n"), Add("story/z.yaml", "schema: story\nid: z\nrules:\n  - id: say\n    when: habit\n    do: []\n")]),
    ("order_lints", Docks, &[Sub(VERBS, "check: grit", "check: courage"), Sub(CHECKS, "stat: grit", "stat: luck"), Sub(VERBS, UNWIELD, "      - meter:\n          who: nobody\n          key: luck\n          add: \"self.zeal\"\n      - reveal:\n          who: world\n          fact: fact_nope\n      - item:\n          who: self\n          id: whale_oil\n          add: 1\n          resist: same"), Sub(SHOP, "  - to: docks_street\n", "  - to: nowhere\n")]),
    ("order_warnings", Docks, &[Sub(VERBS, "          via: target", "          to: docks_street\n          teleport: true"), Add("world/places/attic.md", "---\nschema: place\nid: attic\ndisplay_name: Attic\nfacts: []\n---\nDust.\n"), Add(STORY, story!("        knows: fact_oil_on_shelf\n", "    do:\n      - narrate:\n          to: here\n          text: Someone learned it.\n"))]),
];

/// One distinguishing substring per diagnostic site in section 3.1 of the plan, matched
/// against `report_line` of every diagnostic in every `load.json` and `check.json`.
const EXPECTED: &[&str] = &[
    // canon/mod.rs and diagnostic.rs.
    "world/places/docks_shop.md: schema: this file is not UTF-8 text",
    "cast/mira.md: schema: this file is not UTF-8 text",
    "story/bad.yaml: this file is not UTF-8 text",
    "rules/checks.yaml: schema: this file could not be read as UTF-8 text",
    "this file has no YAML frontmatter",
    "this file has no closing frontmatter fence",
    "an item id is empty",
    "item 'brass_key' is already defined",
    "a fact id is empty",
    "fact 'fact_oil_on_shelf' is already defined",
    "fact 'fact_room_lamp' is already defined",
    "place 'docks_street' is already defined",
    "cast id 'aldric' is already defined",
    "rules/moves.yaml was replaced by rules/verbs.yaml",
    "pack.yaml is missing",
    "rules/verbs.yaml is missing",
    "rules/checks.yaml is missing",
    "rules/meters.yaml is missing",
    "rules/tick.yaml is missing",
    "rule 'story:s/r' is already defined",
    "checkpoint_every is 0",
    "an exit needs a name and a to",
    "exit 'stairs' is listed twice",
    "fix the exit cost",
    "fix the exit condition",
    "a meter id is empty",
    "has min above base_cap",
    "has base_cap above max",
    "has start outside min and base_cap",
    "meter 'grit' is listed twice",
    "a check id is empty",
    "rolls zero dice",
    "has a die with zero sides",
    "names no meter",
    "check 'grit' is listed twice",
    "set id to a short pack id",
    "set id to a short place id",
    "world/places/docks_street.md:4: display_name is empty",
    "set id to a short cast id",
    "id world is reserved",
    "cast/bram.md:5: display_name is empty",
    "which is not npc or pc_template",
    "the schema tag is missing",
    "an old versioned tag",
    "rules/meters.yaml:1: schema is 'checks', which this file does not use",
    "cast/mira.md:2: schema is 'place', which this file does not use",
    "unknown field 'paws'",
    "missing field 'system'",
    "found integer `3` where a one condition or a list of conditions belongs",
    "rules/meters.yaml:6: this value is not a valid i64",
    "unknown value 'sometimes'",
    "pack.yaml:3: this YAML could not be read",
    "this YAML could not be read",
    // canon/schema.rs.
    "a rule id is empty or is say",
    "when 'often' is not habit",
    "verb is empty or is say",
    "target 'door' is not a target kind",
    "when needs one of verb or change",
    "a meter change needs crosses",
    "crosses 'about 3' needs an operator and a number",
    "a flag change needs becomes",
    "an item change needs gained or lost",
    "a change needs arrive, leave, meter, flag, clock, item, or knows",
    "two conditions under one dash",
    "hour needs from and to in 0..24",
    "op '=>' is not <, <=, ==, >=, or >",
    "a condition cannot roll dice",
    "who 'token' is not a who",
    "who 'subject' is not a who; use self",
    "place: needs an id",
    "expression '2 +' is not a number",
    "expression '0d6' rolls no dice",
    "rolls more than 1000 dice",
    "divides by zero",
    "time 'later' is not a duration",
    "time '2w' is not 30s, 10m, 2h, or 1d",
    "roll.check is empty",
    "a change rule has a cost",
    "lists an outcome without a roll",
    "rolls without a success list",
    "sets partial_at without a partial list",
    "lists partial without partial_at",
    "sets partial_at at or above the target",
    "a meter effect needs add or set, not both",
    "flag key busy is not a flag",
    "location needs to or via",
    "a note needs id, text, and to",
    "a clock effect needs add or start",
    "one effect per dash",
    "resist 'maybe' is not same",
    "two verb rules for 'take' at the same level",
    "rule 'take' is listed twice",
    // canon/check.rs.
    "goes to 'nowhere', which is not a place",
    "has no revealed_by fact",
    "exit 'docks_shop' reveals 'fact_nope', which is not a fact",
    "rule 'unwield' reveals 'fact_nope', which is not a fact",
    "names unknown item 'iron_key'",
    "rules[1]: unknown item 'whale_oil'",
    "uses resist: same before any resist roll",
    "reveals to the world",
    "names check 'courage', which is not a check",
    "expression reads 'world.grit'",
    "expression reads meter 'luck'",
    "expression reads meter 'zeal'",
    "expression names 'nobody', which is not a who",
    "uses stat 'luck', which is not in rules/meters.yaml",
    "writes meter 'luck', which is not in rules/meters.yaml",
    "has no path from docks_shop",
    "verb 'travel' teleports",
    "is in a trigger cycle",
    "leaks a change to a viewer who cannot see it",
    // canon/round_settings.rs.
    "max_npcs_acting is -1, which is below zero",
    "is -5, which is below zero; use a number greater than zero",
    "is 0, which is not a positive interval",
    "idle is 'maybe', which is not on or off",
    "rules/tick.yaml:2: this YAML could not be read",
    // canon/save.rs.
    "lacks meter 'rest'",
    "lists meter 'zeal'",
    "meter 'rest' is outside the rules",
    "lists an empty item id",
    "actor 'aldric' lists item 'lamp_oil' with a count of 0",
    "place 'docks_shop' lists item 'lamp_oil' with a count of 0",
    "actors.aldric.inventory.whale_oil: unknown item",
    "actors.aldric.wielded: unknown item 'sword'",
    "places.docks_shop.items.whale_oil: unknown item",
    "which is not a weapon",
    "without holding it",
];

#[test]
fn canon_matches_the_baseline() {
    let write = match std::env::var("SPOOK_ORACLE").as_deref() {
        Ok("write") => true,
        Ok("check") => false,
        _ => return,
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/refactor/oracle");
    if write {
        let _ = fs::remove_dir_all(&root);
    }
    let mut names = BTreeSet::new();
    let mut lines = String::new();
    let mut failures = Vec::new();
    for (name, base, edits) in CASES {
        assert!(
            names.insert(name.to_string()),
            "case {name} is listed twice"
        );
        let pack = build(name, *base, edits);
        let load = load_canon(&pack);
        let check = check_pack(&pack);
        let rev = pack_rev(&pack);
        let _ = fs::remove_dir_all(&pack);
        for diag in load.as_ref().err().into_iter().flatten().chain(&check) {
            lines.push_str(&diag.report_line());
            lines.push('\n');
        }
        let outputs = [
            (
                "canon.txt",
                load.as_ref().map_or("not loaded\n".to_string(), project),
            ),
            (
                "load.json",
                json(load.as_ref().err().map_or(&[], Vec::as_slice)),
            ),
            ("check.json", json(&check)),
            ("rev.txt", format!("{rev:?}\n")),
        ];
        let dir = root.join(name);
        for (file, text) in outputs {
            let path = dir.join(file);
            if write {
                fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("mkdir {name}: {err}"));
                fs::write(&path, text).unwrap_or_else(|err| panic!("write {name}/{file}: {err}"));
                continue;
            }
            match fs::read_to_string(&path) {
                Ok(old) if old == text => {}
                Ok(old) => failures.push(format!("{name}/{file}: {}", first_diff(&old, &text))),
                Err(err) => failures.push(format!("{name}/{file}: {err}")),
            }
        }
    }
    if !write {
        let found: BTreeSet<String> = fs::read_dir(&root)
            .unwrap_or_else(|err| panic!("read {}: {err}", root.display()))
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        for stale in found.difference(&names) {
            failures.push(format!("{stale}: baseline has no case"));
        }
    }
    for want in EXPECTED {
        if !lines.contains(want) {
            failures.push(format!("no case covers {want:?}"));
        }
    }
    assert!(failures.is_empty(), "oracle:\n{}", failures.join("\n"));
}

fn build(name: &str, base: Base, edits: &[Edit]) -> PathBuf {
    let label = format!("oracle-{name}");
    let pack = match base {
        Docks => copy_pack("oneshot-docks", &label),
        D20 => copy_pack("sample-d20", &label),
        Reference => reference_pack(&label),
        Empty => fresh_dir(&label),
    };
    for edit in edits {
        match edit {
            Sub(file, old, new) => {
                let text = fs::read_to_string(pack.join(file)).unwrap();
                assert!(text.contains(old), "{name}: {file} has no {old:?}");
                put(&pack, file, text.replacen(old, new, 1).as_bytes());
            }
            Add(file, text) => put(&pack, file, text.as_bytes()),
            Raw(file, bytes) => put(&pack, file, bytes),
            Del(file) => fs::remove_file(pack.join(file)).unwrap(),
            Crlf => {
                let mut files = vec![pack.join(PACK)];
                for dir in ["world", "cast", "rules"] {
                    walk(&pack.join(dir), &mut files);
                }
                for path in files {
                    let text = fs::read_to_string(&path).unwrap();
                    fs::write(&path, text.replace('\n', "\r\n")).unwrap();
                }
            }
        }
    }
    pack
}

fn put(pack: &Path, file: &str, bytes: &[u8]) {
    let path = pack.join(file);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, bytes).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// The pack in the `yaml file=` fences of `docs/pack-reference.md`, read the way
/// `crates/spook/tests/integration/pack_reference.rs` reads it.
fn reference_pack(label: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/pack-reference.md");
    let text = fs::read_to_string(path)
        .unwrap()
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let pack = fresh_dir(label);
    let mut rest = text.as_str();
    while let Some(start) = rest.find("```") {
        let (info, tail) = rest[start + 3..].split_once('\n').expect("unclosed fence");
        let end = tail.find("\n```").expect("fence is not closed");
        if let Some(file) = info.strip_prefix("yaml") {
            let file = file
                .trim()
                .strip_prefix("file=")
                .expect("yaml fence needs file=");
            let mut body = tail[..end].to_string();
            if !body.ends_with('\n') {
                body.push('\n');
            }
            put(&pack, file.trim(), body.as_bytes());
        }
        rest = &tail[end + 4..];
    }
    pack
}

fn json(diags: &[Diagnostic]) -> String {
    if diags.is_empty() {
        return "ok\n".to_string();
    }
    serde_json::to_string_pretty(diags).unwrap() + "\n"
}

fn first_diff(old: &str, new: &str) -> String {
    let line = old
        .lines()
        .zip(new.lines())
        .take_while(|(a, b)| a == b)
        .count();
    let shown = |text: &str| text.lines().nth(line).unwrap_or("<end>").to_string();
    format!(
        "line {}: baseline {:?}, now {:?}",
        line + 1,
        shown(old),
        shown(new)
    )
}

/// The loaded canon, every field by name. Later phases may change how a field is read,
/// never what is printed.
fn project(canon: &Canon) -> String {
    let Canon {
        pack_id,
        title,
        system,
        start_place,
        start_pc,
        places,
        cast,
        facts,
        rules,
        checks,
        meters,
        items,
        budget,
        checkpoint_every,
    } = canon;
    let mut out = format!(
        "pack_id: {pack_id:?}\ntitle: {title:?}\nsystem: {system:?}\nstart_place: {start_place:?}\nstart_pc: {start_pc:?}\ncheckpoint_every: {checkpoint_every}\n"
    );
    out.push_str("places:\n");
    for (id, place) in places {
        let PlaceCanon {
            name,
            facts,
            body,
            exits,
        } = place;
        out.push_str(&format!(
            "  {id:?}:\n    name: {name:?}\n    facts: {facts:?}\n    body: {body:?}\n"
        ));
        for exit in exits {
            let Exit {
                name,
                to,
                minutes,
                cost,
                if_,
                locked_text,
                hidden,
                revealed_by,
                file,
                index,
            } = exit;
            out.push_str(&format!(
                "    exit {index} in {file:?}: name {name:?} to {to:?} minutes {minutes} hidden {hidden} revealed_by {revealed_by:?} locked_text {locked_text:?}\n    cost: {cost:#?}\n    if: {if_:#?}\n"
            ));
        }
    }
    out.push_str("cast:\n");
    for (id, member) in cast {
        let CastCanon {
            kind,
            display_name,
            facts,
            body,
            tick,
        } = member;
        out.push_str(&format!(
            "  {id:?}:\n    kind: {kind:?}\n    display_name: {display_name:?}\n    facts: {facts:?}\n    body: {body:?}\n    tick: {tick:?}\n"
        ));
    }
    out.push_str("facts:\n");
    for (id, fact) in facts {
        let FactCanon {
            text,
            hidden,
            owner_id,
        } = fact;
        out.push_str(&format!(
            "  {id:?}: text {text:?} hidden {hidden} owner_id {owner_id:?}\n"
        ));
    }
    out.push_str("items:\n");
    for (id, item) in items {
        let ItemCanon { name, tags, body } = item;
        out.push_str(&format!(
            "  {id:?}: name {name:?} tags {tags:?} body {body:?}\n"
        ));
    }
    out.push_str("rules:\n");
    for rule in rules.iter() {
        out.push_str(&format!("{rule:#?}\n"));
    }
    out.push_str(&format!(
        "checks: {checks:#?}\nmeters: {meters:#?}\nbudget: {budget:#?}\n"
    ));
    out
}
