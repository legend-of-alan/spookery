use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

pub fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest)
                .unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn stamp_path(label: &str, n: u32) -> PathBuf {
    let thread: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    std::env::temp_dir().join(format!(
        "spook-{label}-{}-{thread}-{}-{n}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

/// A new empty directory. The name includes the thread, so two scenarios can
/// allocate at the same moment.
pub fn fresh_dir(label: &str) -> PathBuf {
    for n in 0..64 {
        let dir = stamp_path(label, n);
        if fs::create_dir(&dir).is_ok() {
            return dir;
        }
    }
    panic!("no fresh dir for {label}");
}

/// A path that is not on disk yet.
pub fn vacant_path(label: &str) -> PathBuf {
    for n in 0..64 {
        let dir = stamp_path(label, n);
        if !dir.exists() {
            return dir;
        }
    }
    panic!("no vacant path for {label}");
}

/// A fresh copy of `packs/oneshot-docks`.
pub fn temp_pack(label: &str) -> PathBuf {
    copy_pack("oneshot-docks", label)
}

/// A fresh copy of `packs/{name}`.
pub fn copy_pack(name: &str, label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = fresh_dir(label);
    copy_dir(&manifest.join("../../packs").join(name), &pack);
    pack
}

pub fn spook(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

#[derive(Debug)]
pub struct Proc {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Arc<Mutex<String>>,
    pub stderr: Arc<Mutex<String>>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn(args: &[&str]) -> Proc {
    spawn_env(args, &[])
}

pub fn spawn_env(args: &[&str], env: &[(&str, &str)]) -> Proc {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"));
    let stdin = child.stdin.take();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let stdout_buf = Arc::new(Mutex::new(String::new()));
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    drain(stdout, Arc::clone(&stdout_buf));
    drain(stderr, Arc::clone(&stderr_buf));
    Proc {
        child,
        stdin,
        stdout: stdout_buf,
        stderr: stderr_buf,
    }
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
}

pub fn text_of(slot: &Arc<Mutex<String>>) -> String {
    slot.lock().unwrap().clone()
}

pub fn pack_arg(pack: &Path) -> String {
    pack.to_str()
        .unwrap_or_else(|| panic!("pack path is not utf-8"))
        .to_string()
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

/// Set the save's `pack_rev` to the pack as it is now, after a step edits the pack.
pub fn stamp_pack_rev(pack: &Path) {
    let now_path = pack.join("state/save.yaml");
    let mut now = spookery_engine::store::load_save(
        &fs::read_to_string(&now_path).unwrap_or_else(|err| panic!("read now: {err}")),
    )
    .unwrap_or_else(|err| panic!("load now: {err:?}"));
    now.pack_rev =
        spookery_engine::canon::pack_rev(pack).unwrap_or_else(|err| panic!("rev: {err}"));
    fs::write(
        &now_path,
        spookery_engine::store::emit_save(&now).unwrap_or_else(|err| panic!("emit: {err}")),
    )
    .unwrap_or_else(|err| panic!("write now: {err}"));
}

/// The workspace root, two folders above the `spook` crate.
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Collect every `.rs` file under `dir`, skipping `tests`, `target` and `fixtures` folders.
pub fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.is_dir() {
        return;
    }
    let entries = fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|err| panic!("read {}: {err}", dir.display()))
            .path();
        let name = path.file_name().unwrap().to_string_lossy();
        if path.is_dir() {
            if name == "tests" || name == "target" || name == "fixtures" {
                continue;
            }
            rust_files(&path, out);
        } else if name.ends_with(".rs") {
            out.push(path);
        }
    }
}
