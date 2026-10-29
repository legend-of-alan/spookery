//! The Spookery table engine: load a pack, run its rules, and commit or verify the journal.

pub(crate) mod canon;
pub(crate) mod diagnostic;
pub(crate) mod draft;
pub(crate) mod error;
pub(crate) mod hash;
pub(crate) mod intent;
pub(crate) mod map;
pub(crate) mod model;
pub(crate) mod round;
pub(crate) mod rules;
pub(crate) mod store;
pub(crate) mod visibility;
pub(crate) mod world;

pub(crate) use model::key;
pub(crate) use model::meter;
pub(crate) use rules::apply;
pub(crate) use rules::check;
pub(crate) use rules::effect;
pub(crate) use rules::expand;
pub(crate) use rules::perceive;
pub(crate) use rules::rule;
pub(crate) use store::commit;
pub(crate) use store::journal;
pub(crate) use store::lock;

pub use canon::{load_canon, pack_rev};
pub use check::Outcome;
pub use diagnostic::{reason_of, Diagnostic, Severity};
pub use error::Reason;
pub use intent::{Intent, IntentArgs, Source, Target, ViewerId};
pub use map::ExitView;
pub use meter::MeterLive;
pub use model::{Message, Now, RunState};
pub use perceive::{ClockView, ItemStack, KnownFact, MeterView, View};
pub use round::{migrate_pack, play_pack_once, verify_pack, verify_quick, RoundOutput};
pub use rule::When;
pub use rules::{GateRefusal, MoveReport, Roll};
pub use store::{emit_save, load_save, lock_pack, record_line, LockGuard, TapeRecord};
pub use visibility::RecipientLine;
pub use world::{
    check_pack, handle_player_move, GmIntentSpec, Heard, MoveOutcome, QueuedMove, World,
    MAX_PLAYER_SEATS,
};

#[cfg(feature = "testing")]
pub mod testing {
    use std::fs;
    use std::path::{Path, PathBuf};

    pub use crate::canon::strip_body;
    pub use crate::model::{
        ActCounters, ActorLive, Clock, MemoryLine, PauseView, PendingAction, PendingKind,
        PlaceLive, Presence,
    };
    pub use crate::round::replay::replay_journal;
    pub use crate::rules::roll::{faces, RollKey};
    pub use crate::store::commit::append_bundle;
    pub use crate::store::lock::{on_reader_wait, write_guard, WriteGuard};
    pub use crate::store::RULES_REV;

    pub const HOST: &[&str] = &["tests/features/seats"];

    pub const WORLD: &[&str] = &[
        "tests/features/catchup",
        "tests/features/clocks",
        "tests/features/commit",
        "tests/features/firewall",
        "tests/features/perceive",
        "tests/features/round",
        "tests/features/rows",
        "tests/features/sample",
        "tests/features/stock",
    ];

    pub mod journal {
        pub use crate::store::journal::{
            bundles, emit_entry, find_live_bundle, format_record_key, genesis_hash, gunzip_bytes,
            gzip_bytes, hash_now, latest_checkpoint, list_round_keys, locate, read_entries,
            read_record_line, read_verified, seal, JournalEntry, RoundRecord,
        };
    }

    impl crate::World {
        pub fn set_presence(&mut self, presence: crate::model::Presence) -> bool {
            self.set_presence_inner(presence)
        }

        pub fn queue_wait(
            &mut self,
            actor_id: &str,
        ) -> Result<crate::QueuedMove, crate::GateRefusal> {
            self.queue_player_move(actor_id, "wait", None)
        }
    }

    pub fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
        for entry in
            fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display()))
        {
            let entry = entry.unwrap();
            let dest = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &dest);
            } else {
                fs::copy(entry.path(), &dest)
                    .unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
            }
        }
    }

    pub fn fresh_dir(label: &str) -> PathBuf {
        let thread: String = format!("{:?}", std::thread::current().id())
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        for n in 0..64 {
            let dir = std::env::temp_dir().join(format!(
                "spook-{label}-{}-{thread}-{}-{n}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
            ));
            if fs::create_dir(&dir).is_ok() {
                return dir;
            }
        }
        panic!("no fresh dir for {label}");
    }

    pub fn temp_pack(label: &str) -> PathBuf {
        copy_pack("oneshot-docks", label)
    }

    pub fn copy_pack(name: &str, label: &str) -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let pack = fresh_dir(label);
        copy_dir(&manifest.join("../../packs").join(name), &pack);
        pack
    }

    pub fn place_of(now: &mut crate::model::Now, actor: &str, place: &str) {
        let from = now.actors.get(actor).unwrap().place().to_string();
        if from == place {
            return;
        }
        if let Some(live) = now.places.get_mut(&from) {
            live.actors.retain(|id| id != actor);
        }
        let actors = &mut now.places.get_mut(place).unwrap().actors;
        actors.push(actor.to_string());
        actors.sort();
        actors.dedup();
        *now.actors.get_mut(actor).unwrap().place_mut() = place.to_string();
    }

    pub fn open(pack: &Path) -> crate::World {
        crate::World::open(pack)
            .unwrap_or_else(|err| panic!("World::open {}: {err}", pack.display()))
    }

    pub fn now_of(pack: &Path) -> crate::model::Now {
        let report = replay_journal(pack, None, None)
            .unwrap_or_else(|err| panic!("replay_journal {}: {err}", pack.display()));
        let yaml = std::str::from_utf8(&report.now_yaml)
            .unwrap_or_else(|err| panic!("utf-8 {}: {err}", pack.display()));
        crate::store::load_save(yaml)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", pack.display()))
    }

    pub fn save_now(pack: &Path, now: &crate::model::Now) {
        let path = pack.join("state/save.yaml");
        let bytes = crate::store::emit_save(now)
            .unwrap_or_else(|err| panic!("emit {}: {err}", path.display()));
        fs::write(&path, bytes).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    pub fn edit_save(pack: &Path, edit: impl FnOnce(&mut crate::model::Now)) {
        let path = pack.join("state/save.yaml");
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let mut now = crate::store::load_save(&text)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
        edit(&mut now);
        now.pack_rev = crate::canon::pack_rev(pack)
            .unwrap_or_else(|err| panic!("rev {}: {err}", pack.display()));
        save_now(pack, &now);
    }

    pub fn stamp_pack_rev(pack: &Path) {
        edit_save(pack, |_| {});
    }

    pub fn split_move(line: &str) -> (&str, Option<&str>) {
        line.split_once(' ')
            .map_or((line, None), |(v, t)| (v, Some(t)))
    }

    pub fn append(path: &Path, extra: &str) {
        let mut text =
            fs::read_to_string(path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        assert!(!text.contains(extra.trim()), "already present in {path:?}");
        if !text.ends_with('\n') && !extra.starts_with('\n') {
            text.push('\n');
        }
        text.push_str(extra);
        fs::write(path, text).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    pub fn insert_nell(pack: &Path) {
        let from = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/cast/pc-nell.md");
        fs::copy(&from, pack.join("cast/pc-nell.md"))
            .unwrap_or_else(|err| panic!("copy {}: {err}", from.display()));
        edit_save(pack, |now| {
            now.actors.insert(
                "pc_nell".to_string(),
                crate::model::ActorLive::Pc {
                    place: "docks_shop".to_string(),
                    flags: Vec::new(),
                    meters: std::collections::BTreeMap::from([
                        (
                            "grit".to_string(),
                            crate::MeterLive {
                                base: 100,
                                offset: 0,
                            },
                        ),
                        ("rest".to_string(), crate::MeterLive { base: 3, offset: 0 }),
                    ]),
                    inventory: std::collections::BTreeMap::new(),
                    wielded: None,
                    known_facts: Vec::new(),
                    visited: vec!["docks_shop".to_string()],
                    act: crate::model::ActCounters::fresh(),
                },
            );
        });
    }

    pub fn read_round(pack: &Path, key: &str) -> serde_json::Value {
        let text = journal::read_record_line(pack, key)
            .unwrap_or_else(|err| panic!("{} {key}: {}", pack.display(), err.detail));
        serde_json::from_str(text.trim_end())
            .unwrap_or_else(|err| panic!("{} {key}: {err}\n{text}", pack.display()))
    }
}
