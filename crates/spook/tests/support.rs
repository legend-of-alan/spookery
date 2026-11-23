use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub(crate) use spookery_engine::testing::*;
use spookery_engine::{component, ecs, entity, ActorId, Meter};

/// Change `actor`'s meter `key` as `edit` says, through the writer.
pub(crate) fn edit_meter(
    now: &mut ecs::World,
    actor: &str,
    key: &str,
    edit: impl FnOnce(&mut Meter),
) {
    let actor = ActorId::from(actor);
    let held = now.actors().get::<Meter>(&actor).get_or_start(key);
    let mut meter = held.unwrap_or_else(|| panic!("{actor} has no {key}"));
    edit(&mut meter);
    let mut actors = now.actors_mut();
    actors
        .get_mut::<Meter>(&actor)
        .set(key, Some(meter))
        .unwrap();
}

/// Give `id` meter `key` at `base`, with no offset, through the writer.
pub(crate) fn put_meter(now: &mut ecs::World, id: &entity::Id, key: &str, base: i64) {
    let meter = Meter { base, offset: 0 };
    now.entities_mut()
        .get_mut::<component::Meter>(id)
        .set(key, Some(meter))
        .unwrap();
}

/// The save at `pack`'s journal tip, as its file holds it.
pub(crate) fn save_of(pack: &Path) -> serde_json::Value {
    serde_json::to_value(&*world_of(pack)).unwrap_or_else(|err| panic!("save: {err}"))
}

/// Edit the save as its file holds it, for a fixture no writer makes: a count set outright, a
/// fact forgotten.
pub(crate) fn edit_as_saved(now: &mut ecs::World, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut save = serde_json::to_value(&**now).unwrap();
    edit(&mut save);
    **now = serde_json::from_value(save).unwrap_or_else(|err| panic!("edited save: {err}"));
}

/// Leave `id`, an actor or a place, holding `count` of `item`, through the writer.
pub(crate) fn put_items(now: &mut ecs::World, id: &entity::Id, item: &str, count: u32) {
    let to_count = |held: Option<&u32>| i64::from(count) - i64::from(held.copied().unwrap_or(0));
    let put = match id {
        entity::Id::Actor(actor) => {
            let delta = to_count(now.actors().get::<component::Item>(actor).get(item));
            (now.actors_mut().get_mut::<component::Item>(actor)).shift(item, delta, None)
        }
        entity::Id::Place(place) => {
            let delta = to_count(now.places().get::<component::Item>(place).get(item));
            (now.places_mut().get_mut::<component::Item>(place)).shift(item, delta, None)
        }
        _ => panic!("{id} holds no items"),
    };
    put.unwrap();
}

/// How long any test waits for an event before it fails.
pub(crate) const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// Awaits `work`, and panics naming `what` if it takes longer than `WAIT_LIMIT`.
pub(crate) async fn expect_within_limit<T>(
    what: &str,
    work: impl std::future::Future<Output = T>,
) -> T {
    tokio::time::timeout(WAIT_LIMIT, work)
        .await
        .unwrap_or_else(|_| panic!("{what} timed out"))
}

/// Runs blocking `work` on tokio's blocking pool, so other scenarios run meanwhile. A panic in
/// `work` panics here.
pub(crate) async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|err| std::panic::resume_unwind(err.into_panic()))
}

pub(crate) async fn run_spook(args: &[&str]) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(args)
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
        .output()
        .await
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

#[derive(Debug)]
pub(crate) struct Process {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Arc<Mutex<String>>,
    pub stderr: Arc<Mutex<String>>,
    /// The threads that copy stdout and stderr into their buffers.
    pub drains: Vec<JoinHandle<()>>,
}

impl Process {
    /// Waits for both pipes to reach EOF and the child to exit, then returns its status,
    /// stdout and stderr. Stdin stays open, so the child decides when it is done.
    pub(crate) async fn wait_with_output(&mut self) -> (ExitStatus, String, String) {
        let drains = std::mem::take(&mut self.drains);
        run_blocking(move || {
            for drain in drains {
                drain
                    .join()
                    .unwrap_or_else(|_| panic!("a drain thread panicked"));
            }
        })
        .await;
        let status = self
            .child
            .wait()
            .unwrap_or_else(|err| panic!("wait: {err}"));
        (status, text_of(&self.stdout), text_of(&self.stderr))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub(crate) fn spawn(args: &[&str]) -> Process {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(args);
    command
        .current_dir(env!("CARGO_TARGET_TMPDIR"))
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
    let drains = vec![
        drain(stdout, Arc::clone(&stdout_buf)),
        drain(stderr, Arc::clone(&stderr_buf)),
    ];
    Process {
        child,
        stdin,
        stdout: stdout_buf,
        stderr: stderr_buf,
        drains,
    }
}

/// Copies everything `reader` yields into `slot` on its own thread, until EOF.
pub(crate) fn drain(
    reader: impl Read + Send + 'static,
    slot: Arc<Mutex<String>>,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        let mut pending = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.extend_from_slice(&buf[..n]);
                    let complete_len = match std::str::from_utf8(&pending) {
                        Err(err) if err.error_len().is_none() => err.valid_up_to(),
                        _ => pending.len(),
                    };
                    let mut text = slot.lock().unwrap_or_else(|err| err.into_inner());
                    text.push_str(&String::from_utf8_lossy(&pending[..complete_len]));
                    pending.drain(..complete_len);
                }
            }
        }
        let mut text = slot.lock().unwrap_or_else(|err| err.into_inner());
        text.push_str(&String::from_utf8_lossy(&pending));
    })
}

pub(crate) fn text_of(slot: &Arc<Mutex<String>>) -> String {
    slot.lock().unwrap().clone()
}

/// Split the two-way line `a <-> b` in the pack's map, which has no fields, into `a -> b` and
/// `b -> a`, each followed by its own `fields`, indented YAML lines or `""`.
pub(crate) fn split_path(pack: &Path, [a, b]: [&str; 2], [a_fields, b_fields]: [&str; 2]) {
    let path = pack.join("world/map.yaml");
    let text = std::fs::read_to_string(&path).unwrap();
    let line = format!("  - path: {a} <-> {b}\n");
    assert!(text.contains(&line), "no {line} in the map");
    let split = format!("  - path: {a} -> {b}\n{a_fields}  - path: {b} -> {a}\n{b_fields}");
    std::fs::write(&path, text.replacen(&line, &split, 1)).unwrap();
}

/// Insert `block`, whole top-level keys such as `rules:`, at the end of the frontmatter of the
/// Markdown file at `path`.
pub(crate) fn insert_before_fence(path: &Path, block: &str) {
    let text = std::fs::read_to_string(path).unwrap();
    let index = text
        .find("\n---\n")
        .unwrap_or_else(|| panic!("no fence in {}", path.display()));
    let mut next = text[..index].to_string();
    if !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(block);
    if !block.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&text[index + 1..]);
    std::fs::write(path, next).unwrap();
}

pub(crate) fn pack_arg(pack: &Path) -> String {
    pack.to_str()
        .unwrap_or_else(|| panic!("pack path is not utf-8"))
        .to_string()
}
