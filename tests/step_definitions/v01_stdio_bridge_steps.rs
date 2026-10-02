use std::fs;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use cucumber::{given, then, when};
use serde_json::Value;

use crate::support::{self, Proc};
use crate::World;

// Procs cannot live on World. One scenario runs at a time.
// Fields drop in declaration order: refused, bridge, player, server, then the port lock.
struct Hold {
    refused: Option<Proc>,
    bridge: Option<Proc>,
    // Held so Drop kills the player, the shard, and the port lock.
    #[allow(dead_code)]
    player: Proc,
    #[allow(dead_code)]
    server: Proc,
    #[allow(dead_code)]
    lock: support::PortLock,
    pack: PathBuf,
    seated: Vec<u8>,
    perceive: Option<Value>,
    status: Option<Value>,
    refused_code: Option<Option<i32>>,
    refused_stdout: String,
    refused_stderr: String,
}

static HOLD: Mutex<Option<Hold>> = Mutex::new(None);

fn lock_hold() -> std::sync::MutexGuard<'static, Option<Hold>> {
    HOLD.lock().unwrap_or_else(|err| err.into_inner())
}

fn drop_hold() {
    let hold = lock_hold().take();
    let Some(hold) = hold else {
        return;
    };
    let pack = hold.pack.clone();
    drop(hold);
    let _ = fs::remove_dir_all(pack);
}

fn assign_job(child: &Child) {
    #[cfg(windows)]
    windows_job::assign(child);
    #[cfg(not(windows))]
    let _ = child;
}

fn structured(line: &str) -> Value {
    let value: Value = serde_json::from_str(line).unwrap_or_else(|err| panic!("{err}: {line}"));
    value
        .pointer("/result/structuredContent")
        .cloned()
        .unwrap_or_else(|| panic!("no structured content in {line}"))
}

fn assert_seated(w: &World) {
    let pack = w.pack.as_ref().expect("no stdio bridge pack");
    let now =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let hold = lock_hold();
    let seated = &hold.as_ref().expect("no stdio bridge shard").seated;
    assert_eq!(now, *seated);
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

#[given(expr = "a listening shard with a player who said for the bridge")]
fn player_said(w: &mut World) {
    drop_hold();
    let lock = support::lock_port();
    let pack = support::temp_pack("stdio");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let mut player = support::spawn(&["play", &support::pack_arg(&pack)]);
    assign_job(&player.child);
    support::wait_has(
        &player.stdout,
        "\"kind\":\"perceive\"",
        Duration::from_secs(5),
    );
    support::write_stdin(&mut player, "say for the bridge");
    support::wait_has(
        &player.stdout,
        "\"text\":\"for the bridge\"",
        Duration::from_secs(5),
    );
    let seated =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    w.pack = Some(pack.clone());
    *lock_hold() = Some(Hold {
        refused: None,
        bridge: None,
        player,
        server,
        lock,
        pack,
        seated,
        perceive: None,
        status: None,
        refused_code: None,
        refused_stdout: String::new(),
        refused_stderr: String::new(),
    });
}

#[when(expr = "the stdio bridge calls perceive as player:pc_rowan")]
fn bridge_perceive(w: &mut World) {
    let _ = w;
    let mut guard = lock_hold();
    let hold = guard.as_mut().expect("no stdio bridge shard");
    let mut bridge = support::spawn(&["gm", &support::pack_arg(&hold.pack), "--stdio"]);
    assign_job(&bridge.child);
    support::write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"proof","version":"0"}}}"#,
    );
    support::wait_has(&bridge.stdout, "\"id\":1", Duration::from_secs(5));
    support::write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    );
    support::write_stdin(
        &mut bridge,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"perceive","arguments":{"as":"player:pc_rowan"}}}"#,
    );
    let perceived = support::wait_has(&bridge.stdout, "\"id\":2", Duration::from_secs(5));
    let perceive_line = perceived
        .lines()
        .find(|line| line.contains("\"id\":2"))
        .unwrap_or_else(|| panic!("no perceive id 2 in {perceived}"));
    hold.perceive = Some(structured(perceive_line));
    hold.bridge = Some(bridge);
}

#[then(expr = "the bridge perceive is kind perceive for viewer player:pc_rowan with a place")]
fn perceive_view(w: &mut World) {
    let _ = w;
    let guard = lock_hold();
    let view = guard
        .as_ref()
        .expect("no stdio bridge shard")
        .perceive
        .as_ref()
        .expect("no perceive");
    assert_eq!(view["kind"], "perceive");
    assert_eq!(view["viewer"], "player:pc_rowan");
    assert!(view["place"].is_string(), "{view}");
}

#[then(expr = "the bridge now.yaml matches the seated bytes")]
fn now_unchanged(w: &mut World) {
    assert_seated(w);
}

#[when(expr = "the stdio bridge calls session_status since 0")]
fn bridge_status(w: &mut World) {
    let _ = w;
    let mut guard = lock_hold();
    let hold = guard.as_mut().expect("no stdio bridge shard");
    let bridge = hold.bridge.as_mut().expect("no stdio bridge");
    support::write_stdin(
        bridge,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"session_status","arguments":{"since":0}}}"#,
    );
    let status_out = support::wait_has(&bridge.stdout, "\"id\":3", Duration::from_secs(5));
    let status_line = status_out
        .lines()
        .find(|line| line.contains("\"id\":3"))
        .unwrap_or_else(|| panic!("no session_status id 3 in {status_out}"));
    hold.status = Some(structured(status_line));
}

#[then(expr = "session_status last_seq is at least 1 and events include say for the bridge")]
fn status_has_say(w: &mut World) {
    let _ = w;
    let guard = lock_hold();
    let status = guard
        .as_ref()
        .expect("no stdio bridge shard")
        .status
        .as_ref()
        .expect("no session_status");
    assert!(status["last_seq"].as_u64().unwrap() >= 1, "{status}");
    let events = status["events"].as_array().unwrap();
    assert!(
        events.iter().any(|event| {
            event["kind"] == "say"
                && event["from"] == "player:pc_rowan"
                && event["text"] == "for the bridge"
        }),
        "{status}"
    );
}

#[then(expr = "the stdio bridge is still running")]
fn bridge_running(w: &mut World) {
    let _ = w;
    let mut guard = lock_hold();
    let bridge = guard
        .as_mut()
        .expect("no stdio bridge shard")
        .bridge
        .as_mut()
        .expect("no stdio bridge");
    assert!(bridge.child.try_wait().unwrap().is_none(), "bridge exited");
}

#[when(expr = "a second stdio bridge hello is refused")]
fn second_bridge(w: &mut World) {
    let _ = w;
    let mut guard = lock_hold();
    let hold = guard.as_mut().expect("no stdio bridge shard");
    let mut refused = support::spawn(&["gm", &support::pack_arg(&hold.pack), "--stdio"]);
    assign_job(&refused.child);
    let status = support::wait_exit(&mut refused, Duration::from_secs(5));
    hold.refused_code = Some(status.code());
    hold.refused_stdout = support::text_of(&refused.stdout);
    hold.refused_stderr = support::text_of(&refused.stderr);
    hold.refused = Some(refused);
}

#[then(expr = "the refused stdio bridge exits 1 with SESSION_FULL on stderr and empty stdout")]
fn refused_hello(w: &mut World) {
    let _ = w;
    let guard = lock_hold();
    let hold = guard.as_ref().expect("no stdio bridge shard");
    assert_eq!(hold.refused_code, Some(Some(1)));
    assert!(hold
        .refused_stderr
        .contains("{\"kind\":\"refused\",\"code\":\"SESSION_FULL\"}"));
    assert!(
        !hold.refused_stdout.contains("jsonrpc"),
        "{}",
        hold.refused_stdout
    );
    assert!(hold.refused_stdout.is_empty(), "{}", hold.refused_stdout);
}

#[then(expr = "the stdio bridge shard stops")]
fn stop(w: &mut World) {
    drop_hold();
    w.pack = None;
}
