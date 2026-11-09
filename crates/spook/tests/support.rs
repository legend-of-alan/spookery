use std::io::Read;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub use spookery_engine::testing::*;

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
    drain(stdout, Arc::clone(&stdout_buf));
    drain(stderr, Arc::clone(&stderr_buf));
    Process {
        child,
        stdin,
        stdout: stdout_buf,
        stderr: stderr_buf,
    }
}

/// Copies everything `reader` yields into `slot` on its own thread, until EOF.
pub fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 256];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
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

/// Give actor `to` a copy of everything actor `from` holds, standing in `place`.
pub fn copy_actor(state: &mut spookery_engine::State, from: &str, to: &str, place: &str) {
    let (from, to) = (
        spookery_engine::ActorId::from(from),
        spookery_engine::ActorId::from(to),
    );
    // Every store, with no `..`: a store added later must be copied here to compile.
    let ActorComponents {
        kind,
        place: at,
        wielded,
        known_facts,
        visited,
        acts,
        items,
        meters,
        modifiers,
        flags,
    } = &mut state.actors;
    let copied = *kind.get(&from).unwrap_or_else(|| panic!("no actor {from}"));
    kind.insert(to.clone(), copied);
    at.insert(to.clone(), place.to_string());
    if let Some(act) = acts.get(&from).cloned() {
        acts.insert(to.clone(), act);
    }
    if let Some(item) = wielded.get(&from).cloned() {
        wielded.insert(to.clone(), item);
    }
    if let Some(known) = known_facts.get(&from).cloned() {
        known_facts.insert(to.clone(), known);
    }
    copy_owned(visited, &from, &to);
    copy_owned(items, &from, &to);
    copy_owned(meters, &from, &to);
    copy_owned(modifiers, &from, &to);
    let copied: Vec<_> = flags
        .iter()
        .filter(|(owner, _)| *owner == from)
        .map(|(_, flag)| (to.clone(), flag.clone()))
        .collect();
    flags.extend(copied);
}

/// Copy `from`'s entries in a store keyed by `(actor, name)` to `to`.
fn copy_owned<V: Clone>(
    store: &mut std::collections::BTreeMap<(spookery_engine::ActorId, String), V>,
    from: &spookery_engine::ActorId,
    to: &spookery_engine::ActorId,
) {
    let copied: Vec<_> = store
        .iter()
        .filter(|((owner, _), _)| owner == from)
        .map(|((_, name), value)| ((to.clone(), name.clone()), value.clone()))
        .collect();
    store.extend(copied);
}

/// Take actor `id` and everything it holds out of `state`.
pub fn remove_actor(state: &mut spookery_engine::State, id: &str) {
    // Every store, with no `..`: a store added later must be emptied here to compile.
    let ActorComponents {
        kind,
        place,
        wielded,
        known_facts,
        visited,
        acts,
        items,
        meters,
        modifiers,
        flags,
    } = &mut state.actors;
    kind.remove(id);
    place.remove(id);
    acts.remove(id);
    wielded.remove(id);
    known_facts.remove(id);
    visited.retain(|(owner, _), _| *owner != *id);
    items.retain(|(owner, _), _| *owner != *id);
    meters.retain(|(owner, _), _| *owner != *id);
    modifiers.retain(|(owner, _), _| *owner != *id);
    flags.retain(|(owner, _)| *owner != *id);
}
