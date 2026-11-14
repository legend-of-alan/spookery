use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub use spookery_engine::testing::*;
use spookery_engine::{component, entity, ActorId, Meter, State};

/// Change `actor`'s meter `key` as `edit` says, through the writer.
pub fn edit_meter(now: &mut State, actor: &str, key: &str, edit: impl FnOnce(&mut Meter)) {
    let actor = ActorId::from(actor);
    let held = now.actors().get::<Meter>(&actor).get(key);
    let mut meter = held
        .cloned()
        .unwrap_or_else(|| panic!("{actor} has no {key}"));
    edit(&mut meter);
    let mut actors = now.actors_mut();
    actors
        .get_mut::<Meter>(&actor)
        .set(key, Some(meter))
        .unwrap();
}

/// Give `id` meter `key` at `base`, with no offset, through the writer.
pub fn put_meter(now: &mut State, id: &entity::Id, key: &str, base: i64) {
    let meter = Meter { base, offset: 0 };
    now.entities_mut()
        .get_mut::<component::Meter>(id)
        .set(key, Some(meter))
        .unwrap();
}

/// Edit the save as its file holds it, for a fixture no writer makes: a count set outright, a
/// fact forgotten.
pub fn edit_as_saved(now: &mut State, edit: impl FnOnce(&mut serde_json::Value)) {
    let mut save = serde_json::to_value(&**now).unwrap();
    edit(&mut save);
    **now = serde_json::from_value(save).unwrap_or_else(|err| panic!("edited save: {err}"));
}

/// Leave `id`, an actor or a place, holding `count` of `item`, through the writer.
pub fn put_items(now: &mut State, id: &entity::Id, item: &str, count: u32) {
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

/// Run async work from a sync step on the test runtime.
pub fn block_on<T: Send>(work: impl std::future::Future<Output = T> + Send) -> T {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async move {
            tokio::time::timeout(std::time::Duration::from_secs(20), work)
                .await
                .unwrap_or_else(|_| panic!("async work timed out"))
        })
    })
}

pub fn run_spook(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

#[derive(Debug)]
pub struct Process {
    pub child: Child,
    pub stdin: Option<ChildStdin>,
    pub stdout: Arc<Mutex<String>>,
    pub stderr: Arc<Mutex<String>>,
    /// The threads that copy stdout and stderr into their buffers.
    pub drains: Vec<JoinHandle<()>>,
}

impl Process {
    /// Waits for the child to exit and both pipes to reach EOF, then returns its status,
    /// stdout and stderr. Stdin stays open, so the child decides when it is done.
    pub fn wait_with_output(&mut self) -> (ExitStatus, String, String) {
        let status = self
            .child
            .wait()
            .unwrap_or_else(|err| panic!("wait: {err}"));
        for drain in std::mem::take(&mut self.drains) {
            drain
                .join()
                .unwrap_or_else(|_| panic!("a drain thread panicked"));
        }
        (status, text_of(&self.stdout), text_of(&self.stderr))
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn spawn(args: &[&str]) -> Process {
    let mut command = Command::new(env!("CARGO_BIN_EXE_spook"));
    command.args(args);
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
pub fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        // Bytes of a character split across reads wait for the rest of it.
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

pub fn text_of(slot: &Arc<Mutex<String>>) -> String {
    slot.lock().unwrap().clone()
}

pub fn pack_arg(pack: &Path) -> String {
    pack.to_str()
        .unwrap_or_else(|| panic!("pack path is not utf-8"))
        .to_string()
}
