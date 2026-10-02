use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use cucumber::{given, then, when};

use crate::support::{self, Proc};
use crate::World;

// Procs cannot live on World. One scenario runs at a time.
// Fields drop in declaration order: seats die before the port lock.
struct Seat {
    server: Proc,
    player: Option<Proc>,
    gm: Option<Proc>,
    latest: Option<Proc>,
    // Held so the port stays locked until the seat drops.
    #[allow(dead_code)]
    lock: support::PortLock,
    pack: PathBuf,
    files_before: Vec<(PathBuf, Vec<u8>)>,
    now_snapshot: Vec<u8>,
    refusal: String,
    refusals: Vec<String>,
    player_stdout: String,
    player_stderr: String,
    latest_waited: bool,
    latest_code: Option<i32>,
    latest_stdout: String,
    latest_stderr: String,
}

static SEAT: Mutex<Option<Seat>> = Mutex::new(None);

fn lock_seat() -> std::sync::MutexGuard<'static, Option<Seat>> {
    SEAT.lock().unwrap_or_else(|err| err.into_inner())
}

fn with_seat<T>(f: impl FnOnce(&mut Seat) -> T) -> T {
    let mut guard = lock_seat();
    let seat = guard.as_mut().expect("no listening shard");
    f(seat)
}

fn stop_proc(proc: &mut Proc) {
    let _ = proc.child.kill();
    let _ = proc.child.wait();
}

fn drop_seat() {
    let seat = lock_seat().take();
    let Some(mut seat) = seat else {
        return;
    };
    let pack = seat.pack.clone();
    stop_proc(&mut seat.server);
    if let Some(proc) = seat.player.as_mut() {
        stop_proc(proc);
    }
    if let Some(proc) = seat.gm.as_mut() {
        stop_proc(proc);
    }
    if let Some(proc) = seat.latest.as_mut() {
        stop_proc(proc);
    }
    drop(seat);
    let _ = fs::remove_dir_all(pack);
}

fn files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|err| panic!("dirent {}: {err}", dir.display()));
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes =
                    fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
                out.push((path, bytes));
            }
        }
    }
    out.sort();
    out
}

fn tick_files(pack: &Path) -> Vec<String> {
    let log = pack.join("state/log");
    if !log.is_dir() {
        return Vec::new();
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&log).unwrap_or_else(|err| panic!("read {}: {err}", log.display())) {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.starts_with("tick-") && name.ends_with(".jsonl") {
            names.push(name.into_owned());
        }
    }
    names.sort();
    names
}

fn now_of(pack: &Path) -> spookery_kernel::model::Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open: {err}"))
        .now()
        .clone()
}

fn refuse_line(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let n = reader
        .read_line(&mut line)
        .unwrap_or_else(|err| panic!("read refusal: {err}"));
    assert!(n > 0, "connection closed before the refused line");
    let mut extra = [0u8; 1];
    assert_eq!(
        reader
            .read(&mut extra)
            .unwrap_or_else(|err| panic!("read extra: {err}")),
        0,
        "shard left the connection open"
    );
    line
}

fn assign_job(child: &Child) {
    #[cfg(windows)]
    windows_job::assign(child);
    #[cfg(not(windows))]
    let _ = child;
}

#[cfg(windows)]
mod windows_job {
    use std::ffi::c_void;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use std::sync::OnceLock;

    #[repr(C)]
    struct BasicLimit {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        _pad0: u32,
        min_ws: usize,
        max_ws: usize,
        active_process_limit: u32,
        _pad1: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct IoCounters {
        read_op: u64,
        write_op: u64,
        other_op: u64,
        read_xfer: u64,
        write_xfer: u64,
        other_xfer: u64,
    }

    #[repr(C)]
    struct ExtendedLimit {
        basic: BasicLimit,
        io: IoCounters,
        process_memory: usize,
        job_memory: usize,
        peak_process: usize,
        peak_job: usize,
    }

    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            info: *const c_void,
            len: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetLastError() -> u32;
    }

    fn job() -> *mut c_void {
        static JOB: OnceLock<usize> = OnceLock::new();
        let raw = *JOB.get_or_init(|| {
            let handle = unsafe { CreateJobObjectW(std::ptr::null_mut(), std::ptr::null()) };
            assert!(!handle.is_null(), "CreateJobObjectW failed: {}", unsafe {
                GetLastError()
            });
            let mut info = ExtendedLimit {
                basic: BasicLimit {
                    per_process_user_time: 0,
                    per_job_user_time: 0,
                    limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                    _pad0: 0,
                    min_ws: 0,
                    max_ws: 0,
                    active_process_limit: 0,
                    _pad1: 0,
                    affinity: 0,
                    priority_class: 0,
                    scheduling_class: 0,
                },
                io: IoCounters {
                    read_op: 0,
                    write_op: 0,
                    other_op: 0,
                    read_xfer: 0,
                    write_xfer: 0,
                    other_xfer: 0,
                },
                process_memory: 0,
                job_memory: 0,
                peak_process: 0,
                peak_job: 0,
            };
            let ok = unsafe {
                SetInformationJobObject(
                    handle,
                    JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
                    (&mut info as *mut ExtendedLimit).cast(),
                    std::mem::size_of::<ExtendedLimit>() as u32,
                )
            };
            assert!(ok != 0, "SetInformationJobObject failed: {}", unsafe {
                GetLastError()
            });
            handle as usize
        });
        raw as *mut c_void
    }

    pub fn assign(child: &Child) {
        let ok = unsafe { AssignProcessToJobObject(job(), child.as_raw_handle()) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err != 5 {
                panic!("AssignProcessToJobObject failed: {err}");
            }
        }
    }
}

fn capture_exit(client: &mut Proc) -> (Option<i32>, String, String) {
    let status = support::wait_exit(client, Duration::from_secs(5));
    let start = std::time::Instant::now();
    loop {
        let stdout = support::text_of(&client.stdout);
        let stderr = support::text_of(&client.stderr);
        if start.elapsed() > Duration::from_millis(200) {
            return (status.code(), stdout, stderr);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn store_latest(client: Proc, code: Option<i32>, stdout: String, stderr: String) {
    with_seat(|seat| {
        seat.latest = Some(client);
        seat.latest_waited = true;
        seat.latest_code = code;
        seat.latest_stdout = stdout;
        seat.latest_stderr = stderr;
    });
}

fn refusal_line(code: &str) -> String {
    format!("{{\"kind\":\"refused\",\"code\":\"{code}\"}}\n")
}

#[given(expr = "a listening shard of packs\\/oneshot-docks")]
fn listen(w: &mut World) {
    drop_seat();
    let lock = support::lock_port();
    let pack = support::temp_pack("seats");
    let files_before = files(&pack);
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let now_snapshot =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    w.pack = Some(pack.clone());
    *lock_seat() = Some(Seat {
        server,
        player: None,
        gm: None,
        latest: None,
        lock,
        pack,
        files_before,
        now_snapshot,
        refusal: String::new(),
        refusals: Vec::new(),
        player_stdout: String::new(),
        player_stderr: String::new(),
        latest_waited: false,
        latest_code: None,
        latest_stdout: String::new(),
        latest_stderr: String::new(),
    });
}

#[when(expr = "play starts as player:mira")]
fn play_mira(w: &mut World) {
    let pack = support::pack_arg(w.pack.as_ref().unwrap());
    let mut client = support::spawn(&["play", &pack, "--as", "player:mira"]);
    assign_job(&client.child);
    let (code, stdout, stderr) = capture_exit(&mut client);
    store_latest(client, code, stdout, stderr);
}

#[when(expr = "play starts as player:pc_rowan")]
fn play_rowan(w: &mut World) {
    let pack = support::pack_arg(w.pack.as_ref().unwrap());
    let mut client = support::spawn(&["play", &pack, "--as", "player:pc_rowan"]);
    assign_job(&client.child);
    let (code, stdout, stderr) = capture_exit(&mut client);
    store_latest(client, code, stdout, stderr);
}

#[when(expr = "play starts")]
fn play_starts(w: &mut World) {
    let pack = support::pack_arg(w.pack.as_ref().unwrap());
    let client = support::spawn(&["play", &pack]);
    assign_job(&client.child);
    let stdout = support::wait_has(
        &client.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    let stderr = support::text_of(&client.stderr);
    with_seat(|seat| {
        seat.player = Some(client);
        seat.player_stdout = stdout;
        seat.player_stderr = stderr;
    });
}

#[when(expr = "gm starts")]
fn gm_starts(w: &mut World) {
    let pack = support::pack_arg(w.pack.as_ref().unwrap());
    let client = support::spawn(&["gm", &pack]);
    assign_job(&client.child);
    with_seat(|seat| seat.gm = Some(client));
}

#[when(expr = "a second gm client starts")]
fn second_gm(w: &mut World) {
    let pack = support::pack_arg(w.pack.as_ref().unwrap());
    let mut client = support::spawn(&["gm", &pack]);
    assign_job(&client.child);
    let (code, stdout, stderr) = capture_exit(&mut client);
    store_latest(client, code, stdout, stderr);
}

#[when(expr = "the seated now.yaml is recorded")]
fn record_seated_now(w: &mut World) {
    let bytes = fs::read(w.pack.as_ref().unwrap().join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    with_seat(|seat| seat.now_snapshot = bytes);
}

#[when(expr = "a raw hello player:mira is refused")]
fn raw_player_mira(_w: &mut World) {
    let mut raw =
        TcpStream::connect("127.0.0.1:9737").unwrap_or_else(|err| panic!("connect: {err}"));
    raw.set_nodelay(true)
        .unwrap_or_else(|err| panic!("nodelay: {err}"));
    raw.write_all(b"player:mira\n")
        .unwrap_or_else(|err| panic!("write hello: {err}"));
    let line = refuse_line(&mut raw);
    with_seat(|seat| seat.refusal = line);
}

#[when(expr = "a raw second gm is refused")]
fn raw_second_gm(_w: &mut World) {
    std::thread::sleep(Duration::from_millis(300));
    let exited = with_seat(|seat| {
        let gm = seat.gm.as_mut().expect("gm holder");
        gm.child.try_wait().unwrap().map(|status| {
            (
                status.code(),
                support::text_of(&gm.stdout),
                support::text_of(&gm.stderr),
            )
        })
    });
    if let Some((code, stdout, stderr)) = exited {
        panic!("gm holder exited {code:?}\nstdout:\n{stdout}\nstderr:\n{stderr}");
    }
    let mut stream =
        TcpStream::connect("127.0.0.1:9737").unwrap_or_else(|err| panic!("connect: {err}"));
    stream
        .set_nodelay(true)
        .unwrap_or_else(|err| panic!("nodelay: {err}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(b"gm\n")
        .unwrap_or_else(|err| panic!("write hello: {err}"));
    let line = refuse_line(&mut stream);
    with_seat(|seat| seat.refusal = line);
}

#[when(expr = "each bad hello is refused")]
fn bad_hellos(w: &mut World) {
    let pack = w.pack.as_ref().unwrap().clone();
    let snapshot = with_seat(|seat| seat.now_snapshot.clone());
    let mut lines = Vec::new();
    for hello in ["\n", "foo\n", "npc:mira\n", "as=gm\n", "player:\n"] {
        let mut stream =
            TcpStream::connect("127.0.0.1:9737").unwrap_or_else(|err| panic!("connect: {err}"));
        stream
            .set_nodelay(true)
            .unwrap_or_else(|err| panic!("nodelay: {err}"));
        stream
            .write_all(hello.as_bytes())
            .unwrap_or_else(|err| panic!("write hello: {err}"));
        let line = refuse_line(&mut stream);
        assert_eq!(line, refusal_line("SCHEMA"), "{hello:?}");
        let now = fs::read(pack.join("state/now.yaml"))
            .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
        assert_eq!(now, snapshot, "{hello:?}");
        lines.push(line);
    }
    with_seat(|seat| seat.refusals = lines);
}

#[then(expr = "that client exits {int}")]
fn client_exits(_w: &mut World, code: i64) {
    with_seat(|seat| {
        assert!(seat.latest_waited, "client has not exited");
        assert_eq!(seat.latest_code, Some(code as i32));
    });
}

#[then(expr = "that client's stderr is {word}")]
fn client_stderr(_w: &mut World, expected: String) {
    with_seat(|seat| {
        assert_eq!(seat.latest_stderr.trim(), expected);
    });
}

#[then(expr = "that client's stdout is empty")]
fn client_stdout_empty(_w: &mut World) {
    with_seat(|seat| {
        assert!(
            seat.latest_stdout.is_empty(),
            "stdout:\n{}",
            seat.latest_stdout
        );
    });
}

#[then(expr = "the pack bytes are unchanged")]
fn pack_unchanged(w: &mut World) {
    let after = files(w.pack.as_ref().unwrap());
    with_seat(|seat| {
        assert_eq!(after, seat.files_before, "rejected play wrote the pack");
    });
}

#[then(expr = "presence humans is {int}")]
fn humans_are(w: &mut World, n: u64) {
    let now = now_of(w.pack.as_ref().unwrap());
    assert_eq!(now.presence.humans, n as u32);
}

#[then(expr = "the player stdout contains \"kind\":\"perceive\"")]
fn player_perceive(_w: &mut World) {
    with_seat(|seat| {
        assert!(
            seat.player_stdout.contains("\"kind\":\"perceive\""),
            "{}",
            seat.player_stdout
        );
    });
}

#[then(expr = "the player stdout contains \\{\"kind\":\"resume\"")]
fn player_resume(_w: &mut World) {
    with_seat(|seat| {
        assert!(
            seat.player_stdout.contains("{\"kind\":\"resume\""),
            "{}",
            seat.player_stdout
        );
    });
}

#[then(expr = "the player stderr is empty")]
fn player_stderr_empty(_w: &mut World) {
    with_seat(|seat| {
        assert!(seat.player_stderr.is_empty(), "{}", seat.player_stderr);
    });
}

#[then(expr = "actors present are {word}")]
fn actors_present(w: &mut World, id: String) {
    let now = now_of(w.pack.as_ref().unwrap());
    assert_eq!(now.presence.actors_present, vec![id]);
}

#[then(expr = "session_ids length is {int}")]
fn session_len(w: &mut World, n: u64) {
    let now = now_of(w.pack.as_ref().unwrap());
    assert_eq!(now.presence.session_ids.len() as u64, n);
}

#[then(expr = "there is no tick file")]
fn no_tick_file(w: &mut World) {
    assert!(tick_files(w.pack.as_ref().unwrap()).is_empty());
}

#[then(expr = "session.jsonl does not exist")]
fn no_session_tape(w: &mut World) {
    assert!(!w
        .pack
        .as_ref()
        .unwrap()
        .join("state/log/session.jsonl")
        .exists());
}

#[then(expr = "the refusal line is \\{\"kind\":\"refused\",\"code\":\"{word}\"\\}")]
fn one_refusal(_w: &mut World, code: String) {
    let expected = refusal_line(&code);
    with_seat(|seat| {
        assert_eq!(seat.refusal, expected);
    });
}

#[then(expr = "every refusal line is \\{\"kind\":\"refused\",\"code\":\"{word}\"\\}")]
fn every_refusal(_w: &mut World, code: String) {
    let expected = refusal_line(&code);
    with_seat(|seat| {
        assert!(!seat.refusals.is_empty(), "no refusals were recorded");
        for line in &seat.refusals {
            assert_eq!(line, &expected);
        }
    });
}

#[then(expr = "the recorded now.yaml bytes are unchanged")]
fn now_unchanged(w: &mut World) {
    let after = fs::read(w.pack.as_ref().unwrap().join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    with_seat(|seat| {
        assert_eq!(after, seat.now_snapshot);
    });
}

#[then(expr = "the listening shard stops")]
fn stop_shard(_w: &mut World) {
    drop_seat();
}
