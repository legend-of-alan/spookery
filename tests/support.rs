use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct PortLock(std::fs::File);

impl Drop for PortLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub fn lock_port() -> PortLock {
    let path = std::env::temp_dir().join("spookery-9737.lock");
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .unwrap_or_else(|err| panic!("lock {path:?}: {err}"));
    file.lock()
        .unwrap_or_else(|err| panic!("lock {path:?}: {err}"));
    PortLock(file)
}

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

pub fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let pack = std::env::temp_dir().join(format!(
        "spook-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&manifest.join("packs/oneshot-docks"), &pack);
    pack
}

pub fn spook(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

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

pub fn wait_count(
    slot: &Arc<Mutex<String>>,
    needle: &str,
    count: usize,
    timeout: Duration,
) -> String {
    let start = Instant::now();
    loop {
        let text = text_of(slot);
        if text.matches(needle).count() >= count {
            return text;
        }
        if start.elapsed() > timeout {
            panic!(
                "saw {} of {needle}, wanted {count}\n{text}",
                text.matches(needle).count()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_has(slot: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> String {
    let start = Instant::now();
    loop {
        let text = text_of(slot);
        if text.contains(needle) {
            return text;
        }
        if start.elapsed() > timeout {
            panic!("missing {needle} after {timeout:?}\n{text}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_exit(proc: &mut Proc, timeout: Duration) -> std::process::ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = proc.child.try_wait().unwrap() {
            return status;
        }
        if start.elapsed() > timeout {
            panic!(
                "process still running\nstdout:\n{}\nstderr:\n{}",
                text_of(&proc.stdout),
                text_of(&proc.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn wait_listen(proc: &mut Proc) {
    let start = Instant::now();
    loop {
        if TcpStream::connect("127.0.0.1:9737").is_ok() {
            return;
        }
        if let Some(status) = proc.child.try_wait().unwrap() {
            panic!(
                "spook run exited {:?}\nstdout:\n{}\nstderr:\n{}",
                status.code(),
                text_of(&proc.stdout),
                text_of(&proc.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!("listener did not open\nstderr:\n{}", text_of(&proc.stderr));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn release_listeners() {
    let Ok(output) = Command::new("netstat").args(["-ano", "-p", "tcp"]).output() else {
        return;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 4 || fields[3] != "LISTENING" {
            continue;
        }
        let local = fields[1];
        if !(local.ends_with(":9737") || local.ends_with("]:9737")) {
            continue;
        }
        let pid = fields[fields.len() - 1];
        if pid != "0" && !pids.contains(&pid.to_string()) {
            pids.push(pid.to_string());
        }
    }
    for pid in pids {
        if !process_is_spook(&pid) {
            continue;
        }
        let _ = Command::new("taskkill")
            .args(["/PID", &pid, "/T", "/F"])
            .output();
    }
}

fn process_is_spook(pid: &str) -> bool {
    let Ok(listed) = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&listed.stdout)
        .to_ascii_lowercase()
        .contains("spook.exe")
}

pub fn pack_arg(pack: &Path) -> String {
    pack.to_str()
        .unwrap_or_else(|| panic!("pack path is not utf-8"))
        .to_string()
}

pub fn write_stdin(proc: &mut Proc, line: &str) {
    let stdin = proc.stdin.as_mut().unwrap();
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}
