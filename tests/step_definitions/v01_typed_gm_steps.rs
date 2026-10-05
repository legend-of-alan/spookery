use std::fs;
use std::path::PathBuf;
use std::process::Child;
use std::sync::Mutex;
use std::time::Duration;

use cucumber::{given, then, when};

use crate::support::{self, Proc};
use crate::World;

// The typed gm process cannot live on World.
// Fields drop in declaration order: the client, the server, then the port lock.
struct Hold {
    gm: Option<Proc>,
    server: Proc,
    #[allow(dead_code)]
    lock: support::PortLock,
    pack: PathBuf,
    before: Vec<u8>,
}

static HOLD: Mutex<Option<Hold>> = Mutex::new(None);

fn lock_hold() -> std::sync::MutexGuard<'static, Option<Hold>> {
    HOLD.lock().unwrap_or_else(|err| err.into_inner())
}

fn stop_proc(proc: &mut Proc) {
    let _ = proc.child.kill();
    let _ = proc.child.wait();
}

fn drop_hold() {
    let hold = lock_hold().take();
    let Some(mut hold) = hold else {
        return;
    };
    let pack = hold.pack.clone();
    if let Some(proc) = hold.gm.as_mut() {
        stop_proc(proc);
    }
    stop_proc(&mut hold.server);
    drop(hold);
    let _ = fs::remove_dir_all(pack);
}

fn with_hold<T>(f: impl FnOnce(&mut Hold) -> T) -> T {
    let mut guard = lock_hold();
    let hold = guard.as_mut().expect("no typed GM shard");
    f(hold)
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

#[given(expr = "a listening typed GM shard")]
fn listen(w: &mut World) {
    drop_hold();
    let lock = support::lock_port();
    let pack = support::temp_pack("typed-gm");
    let mut server = support::spawn(&["run", &support::pack_arg(&pack)]);
    assign_job(&server.child);
    support::wait_listen(&mut server);
    let before =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    w.pack = Some(pack.clone());
    *lock_hold() = Some(Hold {
        gm: None,
        server,
        lock,
        pack,
        before,
    });
}

#[when(expr = "spook gm runs without --stdio")]
fn run_without_stdio(_w: &mut World) {
    let pack = with_hold(|hold| support::pack_arg(&hold.pack));
    let typed = support::spawn(&["gm", &pack]);
    assign_job(&typed.child);
    with_hold(|hold| hold.gm = Some(typed));
}

#[then(expr = "the typed GM stderr is NOT_IN_SPEC")]
fn stderr_is_not_in_spec(_w: &mut World) {
    let err = with_hold(|hold| {
        let gm = hold.gm.as_ref().expect("no typed gm");
        support::wait_has(&gm.stderr, "NOT_IN_SPEC", Duration::from_secs(5))
    });
    assert_eq!(err.trim(), "NOT_IN_SPEC");
}

#[then(expr = "the typed GM pack has no session.jsonl")]
fn no_tape(_w: &mut World) {
    with_hold(|hold| {
        assert!(!hold.pack.join("state/log/session.jsonl").exists());
    });
}

#[then(expr = "the typed GM now.yaml is unchanged")]
fn now_unchanged(_w: &mut World) {
    with_hold(|hold| {
        let now = fs::read(hold.pack.join("state/now.yaml"))
            .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
        assert_eq!(now, hold.before);
    });
}

#[then(expr = "the typed GM exits 1")]
fn exits_1(_w: &mut World) {
    with_hold(|hold| {
        let gm = hold.gm.as_mut().expect("no typed gm");
        let status = support::wait_exit(gm, Duration::from_secs(5));
        assert_eq!(status.code(), Some(1));
    });
}

#[then(expr = "the typed GM shard stops")]
fn stop(w: &mut World) {
    drop_hold();
    w.pack = None;
}
