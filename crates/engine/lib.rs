//! The Spookery table engine: load a pack, run its rules, and commit or verify the journal.

pub mod canon;
pub mod check;
pub mod diagnostic;
pub mod ecs;
pub(crate) mod hash;
pub mod intent;
pub(crate) mod replay;
pub mod round;
pub(crate) mod rules;
pub(crate) mod session;
pub(crate) mod store;

/// Any entity's id, as rules name it; `ecs::World::entities` reads its components.
pub mod entity {
    pub use crate::ecs::entity::{Id, IdRef, Query, QueryData, QueryFilter, With, Without};
}

/// The report of each verb a round played: its roll or its refusal.
pub mod roll {
    pub use crate::rules::roll::Report;
}

pub use diagnostic::{Code, Diagnostic};
pub use ecs::component;
pub use ecs::component::meter::Meter;
pub use ecs::component::modifier::Modifier;
pub use ecs::component::Note;
pub use ecs::entity::globals;
pub use ecs::id::{ActorId, PathId, PlaceId, StoryId};
pub use ecs::{Presence, Reader, Save, Writer};
pub use intent::{Intent, Source, Target};
pub use round::verification;
pub use rules::gate::Refusal;
pub use rules::perceive::View;
pub use rules::visibility::RecipientLine;
pub use session::{migrate_pack, Session};
pub use spookery_protocol::Reason;
pub use store::save::{emit as emit_save, parse as parse_save};
pub use store::{lock, tape, PackGuard};

#[cfg(feature = "testing")]
pub mod testing {
    use std::fs;
    use std::path::{Path, PathBuf};

    use crate::ecs::component::Place;
    use crate::store::lock::WriteGuard;

    pub use crate::canon::Canon;
    pub use crate::ecs::entity::Actors;
    pub use crate::ecs::{Act, PendingAction};

    pub mod replay {
        pub use crate::replay::apply_journal;
    }

    pub mod roll {
        pub use crate::rules::roll::{faces, Key};
    }

    /// Where `actor` stands, read through `Reader::get`. Panics when they stand nowhere.
    pub fn place_of(world: &crate::ecs::World, actor: &str) -> String {
        let actors = world.actors();
        let place = actors
            .get::<Place>(&actor.into())
            .get()
            .map(ToString::to_string);
        place.unwrap_or_else(|| panic!("{actor} stands nowhere"))
    }

    /// What `actor` is doing, read through `Reader::get`.
    pub fn act_of(world: &crate::ecs::World, actor: &str) -> Act {
        world.actors().get::<Act>(&actor.into()).get().clone()
    }

    /// Whether `actor` is busy at the save's world time.
    pub fn is_busy(world: &crate::ecs::World, actor: &str) -> bool {
        world
            .actors()
            .get::<Act>(&actor.into())
            .is_busy(world.world_time)
    }

    /// The write guard of the pack `session` holds the lock on.
    pub fn lock_write(session: &crate::session::Session) -> Result<WriteGuard, crate::Reason> {
        crate::store::lock::take_write(session.root(), session.lock())
    }
    pub use crate::store::lock::set_read_wait_handler;
    pub use crate::store::RULES_REV;

    /// A second handle on the lock file, like the copy a spawned child holds until it execs.
    pub fn clone_lock_file(session: &crate::session::Session) -> std::fs::File {
        session.lock().file().try_clone().unwrap()
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
        pub fn find_bundle(pack: &Path, key: &str) -> Result<PathBuf, Error> {
            for (bundle, entries) in read_bundle_records(pack)? {
                if entries.iter().any(|entry| format_record_key(entry) == key) {
                    return Ok(pack.join("state/journal").join(bundle));
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

    /// Copy a test pack from `tests/packs` with its dice pinned: the save's seed is its world id.
    pub fn copy_pack(dir: &str, label: &str) -> PathBuf {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let pack = fresh_dir(label);
        copy_dir(&manifest.join("../../tests/packs").join(dir), &pack);
        let text = fs::read_to_string(crate::store::journal::save_path(&pack))
            .unwrap_or_else(|err| panic!("read {dir} save: {err}"));
        let mut save = crate::store::save::parse(&text)
            .unwrap_or_else(|err| panic!("load {dir} save: {err:?}"));
        save.seed.clone_from(&save.world_id);
        write_save(&pack, &save);
        pack
    }

    /// Lock the pack and open its save, for a test that plays rounds on it.
    pub fn resume(pack: &Path) -> crate::session::Session {
        let lock = crate::lock::take(pack)
            .unwrap_or_else(|err| panic!("lock::take {}: {err}", pack.display()));
        crate::session::Session::resume(pack, lock)
            .unwrap_or_else(|err| panic!("Session::resume {}: {err}", pack.display()))
    }

    /// Play one round at an empty table, timed at the save's last round.
    pub fn play_empty_round(pack: &Path) {
        let mut session = resume(pack);
        let wall = session.world().last_round_at.clone();
        let played = session.play(&crate::ecs::Presence::default(), &wall);
        played.unwrap_or_else(|err| panic!("round {}: {err}", pack.display()));
    }

    /// Read the pack's journal and replay all of it.
    pub fn replay_pack(pack: &Path) -> Result<crate::replay::Report, crate::replay::Error> {
        let records = crate::store::journal::read_records(pack)?;
        replay::apply_journal(pack, &records, None, None)
    }

    /// The ECS world the pack's journal replays to, read against the pack's canon.
    pub fn world_of(pack: &Path) -> crate::ecs::World {
        let report =
            replay_pack(pack).unwrap_or_else(|err| panic!("replay_pack {}: {err}", pack.display()));
        let yaml = std::str::from_utf8(&report.now_yaml)
            .unwrap_or_else(|err| panic!("utf-8 {}: {err}", pack.display()));
        let save = crate::store::save::parse(yaml)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", pack.display()));
        let canon = crate::canon::load(pack)
            .unwrap_or_else(|err| panic!("canon {}: {err:?}", pack.display()));
        crate::ecs::World::new(save, std::sync::Arc::new(canon))
    }

    pub fn write_save(pack: &Path, save: &crate::ecs::Save) {
        let path = crate::store::journal::save_path(pack);
        let bytes = crate::store::save::emit(save)
            .unwrap_or_else(|err| panic!("emit {}: {err}", path.display()));
        fs::write(&path, bytes).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    }

    /// Edit the pack's save through the writers, with no round open, on the pack's canon, and
    /// write it back stamped with the pack's revision.
    pub fn edit_save(pack: &Path, edit: impl FnOnce(&mut crate::ecs::World)) {
        let path = crate::store::journal::save_path(pack);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let save = crate::store::save::parse(&text)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
        let canon = crate::canon::load(pack)
            .unwrap_or_else(|err| panic!("canon {}: {err:?}", pack.display()));
        let mut world = crate::ecs::World::new(save, std::sync::Arc::new(canon));
        edit(&mut world);
        world.accept();
        write_save(pack, &world);
        stamp_pack_rev(pack);
    }

    /// Stamp the save with the pack's revision, as its file holds it: the canon need not load.
    pub fn stamp_pack_rev(pack: &Path) {
        let path = crate::store::journal::save_path(pack);
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let mut save = crate::store::save::parse(&text)
            .unwrap_or_else(|err| panic!("load {}: {err:?}", path.display()));
        save.pack_rev =
            crate::canon::hash(pack).unwrap_or_else(|err| panic!("rev {}: {err}", pack.display()));
        write_save(pack, &save);
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
        edit_save(pack, |world| {
            let nell = crate::ActorId::from("nell");
            let mut actors = world.actors_mut();
            actors.add_actor(&nell, crate::component::ControlType::Pc, "docks_shop");
            for (key, base) in [("hp", 8), ("might", 1), ("rest", 3), ("vigor", 100)] {
                let meter = crate::Meter { base, offset: 0 };
                let set = (actors.get_mut::<crate::Meter>(&nell)).set(key, Some(meter));
                set.unwrap();
            }
        });
    }
}
