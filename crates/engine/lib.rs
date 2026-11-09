//! The Spookery table engine: load a pack, run its rules, and commit or verify the journal.

pub(crate) mod canon;
pub(crate) mod diagnostic;
pub(crate) mod draft;
pub(crate) mod entity;
pub(crate) mod hash;
pub(crate) mod intent;
pub(crate) mod round;
pub(crate) mod rules;
pub(crate) mod state;
pub(crate) mod store;
pub(crate) mod world;

pub use canon::{load_canon, pack_rev};
pub use diagnostic::{first_reason, Code, Diagnostic};
pub use entity::Entity;
pub use intent::{Intent, IntentArgs, Source, Target, Viewer};
pub use round::{migrate_pack, play_pack_once, verify_pack, verify_quick, RoundOutput};
pub use rules::gate::Refusal;
pub use rules::perceive::View;
pub use rules::roll::MoveReport;
pub use rules::visibility::RecipientLine;
pub use spookery_protocol::Reason;
pub use state::id::{ActorId, EntityId, PlaceId, StoryId};
pub use state::meter::Meter;
pub use state::modifier::Modifier;
pub use state::{Note, Presence, State};
pub use store::save::{emit as emit_save, parse as parse_save};
pub use store::{emit_tape_record, lock_pack, PackGuard, TapeRecord};
pub use world::{check_pack, Address, World};

#[cfg(feature = "testing")]
pub mod testing {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::store::lock::WriteGuard;

    pub use crate::canon::strip_body;
    pub use crate::round::replay::replay_journal;
    pub use crate::rules::roll::{faces, RollKey};
    pub use crate::state::{Act, ActorComponents, ActorKind, PendingAction};

    /// The write guard of the pack `world` holds the lock on.
    pub fn lock_write(world: &crate::World) -> Result<WriteGuard, crate::Reason> {
        crate::store::lock::lock_write(world.root(), &world.lock)
    }
    pub use crate::store::RULES_REV;

    /// A second handle on the lock file, like the copy a spawned child holds until it execs.
    pub fn clone_lock_file(world: &crate::World) -> std::fs::File {
        world.lock.file().try_clone().unwrap()
    }

    pub mod journal {
        use std::path::{Path, PathBuf};

        use crate::store::journal::{read_bundle_records, Error};
        use crate::Reason;

        pub use crate::hash::blake2s_tagged;
        pub use crate::store::journal::{
            emit_record, find_live_bundle, gunzip_bytes, gzip_bytes, latest_checkpoint,
            list_bundles, read_records, read_verified,
        };
        pub use crate::store::records::Record;

        /// Keys of every round record, such as `round-0001`, in order.
        pub fn list_round_keys(pack: &Path) -> Vec<String> {
            let Ok(entries) = read_records(pack) else {
                return Vec::new();
            };
            let mut keys: Vec<String> = entries
                .iter()
                .filter(|entry| matches!(entry, Record::Round(_)))
                .map(format_record_key)
                .collect();
            keys.sort();
            keys
        }

        /// The stored line for a record key such as `round-0001` or `migrate-0002`, without a newline.
        pub fn read_record_line(pack: &Path, key: &str) -> Result<String, Error> {
            let entries = read_records(pack)?;
            let found = entries
                .into_iter()
                .find(|entry| format_record_key(entry) == key);
            match found {
                Some(entry) => Ok(emit_record(&entry).trim_end().to_string()),
                None => Err(Error::new(
                    Path::new(key),
                    Reason::Schema,
                    "the journal record is missing",
                )),
            }
        }

        /// The bundle a logical record lives in, so a test can edit that line.
        pub fn locate(pack: &Path, key: &str) -> Result<PathBuf, Error> {
            for (name, entries) in read_bundle_records(pack)? {
                if entries.iter().any(|entry| format_record_key(entry) == key) {
                    return Ok(pack.join("state/journal").join(name));
                }
            }
            Err(Error::new(
                Path::new(key),
                Reason::Schema,
                "the journal record is missing",
            ))
        }

        /// Key naming one record inside the bundles: `round-0001`, `migrate-0002` or `checkpoint-0003`.
        pub fn format_record_key(entry: &Record) -> String {
            match entry {
                Record::Round(record) => format!("round-{:04}", record.round_index),
                Record::Migrate(record) => format!("migrate-{:04}", record.state_rev_after),
                Record::Checkpoint(record) => format!("checkpoint-{:04}", record.round_index),
            }
        }

        pub fn state_hash(entry: &Record) -> &str {
            match entry {
                Record::Round(record) => &record.state_hash,
                Record::Migrate(record) => &record.state_hash,
                Record::Checkpoint(record) => &record.state_hash,
            }
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

    pub fn copy_docks(label: &str) -> PathBuf {
        copy_pack("oneshot-docks", label)
    }

    pub fn copy_pack(name: &str, label: &str) -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let pack = fresh_dir(label);
        copy_dir(&manifest.join("../../packs").join(name), &pack);
        pack
    }

    /// Lock the pack and open its save, for a test that plays rounds on it.
    pub fn resume(pack: &Path) -> crate::World {
        let lock = crate::lock_pack(pack)
            .unwrap_or_else(|err| panic!("lock_pack {}: {err}", pack.display()));
        crate::World::resume(pack, lock)
            .unwrap_or_else(|err| panic!("World::resume {}: {err}", pack.display()))
    }

    /// Read the pack's journal and replay all of it.
    pub fn replay_pack(
        pack: &Path,
    ) -> Result<crate::round::replay::Report, crate::round::replay::Error> {
        let records = crate::store::journal::read_records(pack)?;
        replay_journal(pack, &records, None, None)
    }

    pub fn state_of(pack: &Path) -> crate::state::State {
        let report =
            replay_pack(pack).unwrap_or_else(|err| panic!("replay_pack {}: {err}", pack.display()));
        let yaml = std::str::from_utf8(&report.now_yaml)
            .unwrap_or_else(|err| panic!("utf-8 {}: {err}", pack.display()));
        crate::store::save::parse(yaml)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", pack.display()))
    }

    pub fn write_save(pack: &Path, state: &crate::state::State) {
        let path = crate::store::journal::save_path(pack);
        let bytes = crate::store::save::emit(state)
            .unwrap_or_else(|err| panic!("emit {}: {err}", path.display()));
        fs::write(&path, bytes).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    pub fn edit_save(pack: &Path, edit: impl FnOnce(&mut crate::state::State)) {
        let path = crate::store::journal::save_path(pack);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let mut state = crate::store::save::parse(&text)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
        edit(&mut state);
        state.pack_rev = crate::canon::pack_rev(pack)
            .unwrap_or_else(|err| panic!("rev {}: {err}", pack.display()));
        write_save(pack, &state);
    }

    pub fn stamp_pack_rev(pack: &Path) {
        edit_save(pack, |_| {});
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
        edit_save(pack, |state| {
            let nell = crate::ActorId::from("pc_nell");
            let actors = &mut state.actors;
            actors
                .kind
                .insert(nell.clone(), crate::state::ActorKind::Pc);
            actors.place.insert(nell.clone(), "docks_shop".to_string());
            actors
                .acts
                .insert(nell.clone(), crate::state::Act::default());
            let grit = crate::Meter {
                base: 100,
                offset: 0,
            };
            let rest = crate::Meter { base: 3, offset: 0 };
            actors
                .meters
                .insert((nell.clone(), "grit".to_string()), grit);
            actors.meters.insert((nell, "rest".to_string()), rest);
        });
    }

    pub fn read_round(pack: &Path, key: &str) -> serde_json::Value {
        let text = journal::read_record_line(pack, key)
            .unwrap_or_else(|err| panic!("{} {key}: {}", pack.display(), err.detail));
        serde_json::from_str(text.trim_end())
            .unwrap_or_else(|err| panic!("{} {key}: {err}\n{text}", pack.display()))
    }
}
