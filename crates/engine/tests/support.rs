use std::fs;
use std::path::{Path, PathBuf};

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
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

pub fn place_of(now: &mut spookery_engine::model::Now, actor: &str, place: &str) {
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

pub fn stamp_pack_rev(pack: &Path) {
    let now_path = pack.join("state/save.yaml");
    let mut now =
        spookery_engine::store::load_save(&fs::read_to_string(&now_path).unwrap()).unwrap();
    now.pack_rev = spookery_engine::canon::pack_rev(pack).unwrap();
    fs::write(&now_path, spookery_engine::store::emit_save(&now).unwrap()).unwrap();
}
