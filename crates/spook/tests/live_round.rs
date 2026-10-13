//! One real round against a live chat-completions provider.
//!
//! Ignored by default. It needs `SPOOK_GM_KEY`, `SPOOK_GM_URL`, and
//! `SPOOK_GM_MODEL`, the same three settings `spook ai-gm` reads:
//!
//! ```sh
//! SPOOK_GM_KEY=ollama SPOOK_GM_URL=http://localhost:11434/v1 SPOOK_GM_MODEL=qwen2.5:0.5b \
//!   cargo test -p spook --test live_round --locked -- --ignored --nocapture
//! ```
//!
//! The test hosts a copy of the docks pack with `spook run`, sits `spook ai-gm`,
//! and queues one player move. A small pass-through proxy sits between
//! `ai-gm` and the provider so the test can see each model call and its HTTP
//! status. The round passes when every model call returned 2xx, `ai-gm`
//! printed no model-call failure, stayed up, and the queued move committed one
//! tick. What the model says does not matter.

#[path = "support.rs"]
#[allow(dead_code)]
mod support;

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use support::{pack_arg, temp_pack, text_of, Proc};

/// Long enough for one call, one retry, and a slow CPU-only model.
const ROUND_BOUND: Duration = Duration::from_secs(240);

struct Call {
    status: u16,
    body: String,
}

#[test]
#[ignore = "needs a live provider: set SPOOK_GM_KEY, SPOOK_GM_URL, SPOOK_GM_MODEL"]
fn a_live_model_plays_one_round() {
    let key = setting("SPOOK_GM_KEY");
    let upstream = setting("SPOOK_GM_URL").trim_end_matches('/').to_string();
    let model = setting("SPOOK_GM_MODEL");
    println!("live round: model {model} at {upstream}");

    let pack = temp_pack("live-round");
    let pack_text = pack_arg(&pack);
    let port = free_port();
    let addr = format!("127.0.0.1:{port}");

    let mut server = support::spawn(&["--addr", &addr, "run", &pack_text]);
    wait_port(&addr, &mut server);

    let (proxy, calls) = proxy(upstream);
    let mut host = spawn_host(&addr, &pack_text, &key, &format!("http://{proxy}"), &model);
    wait_text(&host, &host.stderr, "listening", Duration::from_secs(20));

    let sit = world_time(&pack);
    let mut player = support::spawn(&["--addr", &addr, "play", &pack_text]);
    wait_text(
        &player,
        &player.stdout,
        "You are in",
        Duration::from_secs(20),
    );
    let stdin = player.stdin.as_mut().expect("player stdin");
    stdin
        .write_all(b"travel docks_street\n")
        .expect("send travel");
    stdin.flush().expect("flush travel");

    let advanced = wait_advance(&pack, sit, &mut host);
    let host_err = text_of(&host.stderr);
    let up = host.child.try_wait().ok().flatten().is_none();
    let recorded: Vec<(u16, String)> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|call| (call.status, call.body.clone()))
        .collect();

    for (index, (status, body)) in recorded.iter().enumerate() {
        let acted = body.contains("\"tool_calls\"");
        println!(
            "model call {}: HTTP {status}, tool call: {acted}, reply: {}",
            index + 1,
            short(body)
        );
    }

    drop(player);
    drop(host);
    drop(server);
    let _ = fs::remove_dir_all(&pack);

    assert!(
        !recorded.is_empty(),
        "ai-gm never called the model\n{host_err}"
    );
    for (status, body) in &recorded {
        assert!(
            (200..300).contains(status),
            "model call returned {status}: {}",
            short(body)
        );
    }
    let failures: Vec<&str> = host_err
        .lines()
        .filter(|line| line.starts_with("model call"))
        .collect();
    assert!(failures.is_empty(), "ai-gm reported {failures:?}");
    assert!(up, "ai-gm exited\n{host_err}");
    assert!(
        advanced,
        "the queued move did not commit a tick\n{host_err}"
    );
    println!(
        "live round passed: {} model call(s), one tick committed",
        recorded.len()
    );
}

fn setting(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("{name} is unset; this test needs a live provider"),
    }
}

fn spawn_host(addr: &str, pack: &str, key: &str, url: &str, model: &str) -> Proc {
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_spook"));
    command
        .args(["--addr", addr, "ai-gm", pack])
        .env("SPOOK_GM_KEY", key)
        .env("SPOOK_GM_URL", url)
        .env("SPOOK_GM_MODEL", model)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|err| panic!("spawn ai-gm: {err}"));
    let stdout = Arc::new(Mutex::new(String::new()));
    let stderr = Arc::new(Mutex::new(String::new()));
    drain(child.stdout.take().unwrap(), Arc::clone(&stdout));
    drain(child.stderr.take().unwrap(), Arc::clone(&stderr));
    Proc {
        child,
        stdin: None,
        stdout,
        stderr,
    }
}

fn drain(reader: impl Read + Send + 'static, slot: Arc<Mutex<String>>) {
    thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 512];
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

/// Forward each POST to `{upstream}/chat/completions` and record the status
/// and reply body. Request headers are never logged, so the key stays out of
/// the output.
fn proxy(upstream: String) -> (String, Arc<Mutex<Vec<Call>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
    let addr = listener.local_addr().unwrap().to_string();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let slot = Arc::clone(&calls);
    thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(180))
            .build()
            .unwrap_or_else(|err| panic!("client: {err}"));
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                break;
            };
            let (auth, body) = read_request(&mut stream);
            let mut request = client
                .post(format!("{upstream}/chat/completions"))
                .header("Content-Type", "application/json")
                .body(body);
            if let Some(auth) = auth {
                request = request.header("Authorization", auth);
            }
            let (status, reply) = match request.send() {
                Ok(response) => {
                    let status = response.status().as_u16();
                    (status, response.text().unwrap_or_default())
                }
                Err(err) => (502, format!("{{\"proxy\":\"{err}\"}}")),
            };
            slot.lock().unwrap().push(Call {
                status,
                body: reply.clone(),
            });
            let header = format!(
                "HTTP/1.1 {status} Proxied\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                reply.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (addr, calls)
}

fn read_request(stream: &mut TcpStream) -> (Option<String>, Vec<u8>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return (None, Vec::new()),
            Ok(_) => buf.push(byte[0]),
        }
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let mut length = 0usize;
    let mut auth = None;
    for line in head.lines() {
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            length = rest.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            auth = Some(line["authorization:".len()..].trim().to_string());
        }
    }
    let mut body = vec![0u8; length];
    let _ = stream.read_exact(&mut body);
    (auth, body)
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap_or_else(|err| panic!("bind: {err}"))
        .local_addr()
        .unwrap()
        .port()
}

fn wait_port(addr: &str, server: &mut Proc) {
    let start = Instant::now();
    while TcpStream::connect(addr).is_err() {
        if let Some(status) = server.child.try_wait().unwrap() {
            panic!("spook run exited {status:?}\n{}", text_of(&server.stderr));
        }
        if start.elapsed() > Duration::from_secs(20) {
            panic!("spook run did not listen");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_text(proc: &Proc, slot: &Arc<Mutex<String>>, needle: &str, bound: Duration) {
    let start = Instant::now();
    while !text_of(slot).contains(needle) {
        if start.elapsed() > bound {
            panic!(
                "missing {needle:?}\nstdout:\n{}\nstderr:\n{}",
                text_of(&proc.stdout),
                text_of(&proc.stderr)
            );
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn world_time(pack: &Path) -> u64 {
    let text = fs::read_to_string(pack.join("state/save.yaml")).expect("save.yaml");
    spookery_kernel::store::codec::load_now(&text)
        .expect("load now")
        .world_time
}

fn wait_advance(pack: &Path, sit: u64, host: &mut Proc) -> bool {
    let start = Instant::now();
    while start.elapsed() < ROUND_BOUND {
        if let Ok(text) = fs::read_to_string(pack.join("state/save.yaml")) {
            if let Ok(now) = spookery_kernel::store::codec::load_now(&text) {
                if now.world_time > sit {
                    return true;
                }
            }
        }
        if host.child.try_wait().ok().flatten().is_some() {
            return false;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

fn short(text: &str) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= 300 {
        return line;
    }
    let mut cut: String = line.chars().take(300).collect();
    cut.push_str("...");
    cut
}
