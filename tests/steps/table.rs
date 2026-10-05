use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use spookery_kernel::model::Now;

use crate::support::{self, Proc};
use crate::World;

pub struct Live {
    pub port: u16,
    pub player_write: Option<TcpStream>,
    pub player_text: Arc<Mutex<String>>,
    pub gm_write: Option<TcpStream>,
    pub gm_text: Arc<Mutex<String>>,
    pub opened_now: Vec<u8>,
    pub seated_now: Vec<u8>,
    pub seated_rev: u64,
    pub server: Option<Proc>,
}

pub fn pack<'a>(world: &'a World) -> &'a Path {
    world
        .pack
        .as_deref()
        .unwrap_or_else(|| panic!("no docks table"))
}

pub fn now_of(pack: &Path) -> Now {
    spookery_kernel::open_shard(pack)
        .unwrap_or_else(|err| panic!("open_shard: {err}"))
        .now()
        .clone()
}

pub fn tick_names(pack: &Path) -> Vec<String> {
    let dir = pack.join("state/log");
    let mut names = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(&dir).unwrap_or_else(|err| panic!("read log: {err}")) {
            let entry = entry.unwrap_or_else(|err| panic!("dirent: {err}"));
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with("tick-") && name.ends_with(".jsonl") {
                names.push(name);
            }
        }
    }
    names.sort();
    names
}

pub fn listen(pack: &Path, bind: &str, port: u16) {
    let pack_arg = support::pack_arg(pack);
    let mut server = support::spawn(&["run", &pack_arg, "--bind", bind]);
    wait_listen(port, &mut server);
    let opened_now =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    crate::set_live(Live {
        port,
        player_write: None,
        player_text: Arc::new(Mutex::new(String::new())),
        gm_write: None,
        gm_text: Arc::new(Mutex::new(String::new())),
        opened_now: opened_now.clone(),
        seated_now: opened_now,
        seated_rev: 0,
        server: Some(server),
    });
}

pub fn seat_player(pack: &Path) {
    let port = crate::with_live(|live| live.port);
    let (write, text) = connect_player(port);
    let seated_now =
        fs::read(pack.join("state/now.yaml")).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    let seated_rev = now_of(pack).state_rev;
    crate::with_live(|live| {
        live.player_write = Some(write);
        live.player_text = text;
        live.seated_now = seated_now;
        live.seated_rev = seated_rev;
    });
}

pub fn seat_gm() {
    let port = crate::with_live(|live| live.port);
    let (write, text) = connect_gm(port);
    crate::with_live(|live| {
        live.gm_write = Some(write);
        live.gm_text = text;
    });
}

pub fn say(line: &str, text: &str) {
    let _ = player_send(line, &format!("\"text\":\"{text}\""));
}

pub fn player_len() -> usize {
    crate::with_live(|live| support::text_of(&live.player_text).len())
}

pub fn player_send(line: &str, needle: &str) -> (usize, String) {
    let (slot, mark) = crate::with_live(|live| {
        let mark = support::text_of(&live.player_text).len();
        let write = live
            .player_write
            .as_mut()
            .unwrap_or_else(|| panic!("no seated player"));
        writeln!(write, "{line}").unwrap_or_else(|err| panic!("write: {err}"));
        write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
        (Arc::clone(&live.player_text), mark)
    });
    (mark, wait_suffix(&slot, mark, needle))
}

pub fn wait_player_from(mark: usize, needle: &str) -> String {
    let slot = crate::with_live(|live| Arc::clone(&live.player_text));
    wait_suffix(&slot, mark, needle)
}

pub fn gm_call(id: u64, name: &str, arguments: &str) -> String {
    let needle = format!("\"id\":{id}");
    let (slot, mark) = crate::with_live(|live| {
        let mark = support::text_of(&live.gm_text).len();
        let write = live
            .gm_write
            .as_mut()
            .unwrap_or_else(|| panic!("no seated GM"));
        let name = serde_json::to_string(name).unwrap_or_else(|err| panic!("name: {err}"));
        writeln!(
            write,
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":{name},\"arguments\":{arguments}}}}}"
        )
        .unwrap_or_else(|err| panic!("gm call: {err}"));
        write.flush().unwrap_or_else(|err| panic!("flush: {err}"));
        (Arc::clone(&live.gm_text), mark)
    });
    wait_suffix(&slot, mark, &needle)
}

pub fn wait_gm(needle: &str) -> String {
    let slot = crate::with_live(|live| Arc::clone(&live.gm_text));
    wait_text(&slot, needle);
    support::text_of(&slot)
}

pub fn wait_world_time(pack: &Path, expected: u64) {
    let start = Instant::now();
    let path = pack.join("state/now.yaml");
    loop {
        let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read now.yaml: {err}"));
        if let Ok(now) = spookery_kernel::codec::load_now(&text) {
            if now.world_time == expected {
                return;
            }
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("world_time did not become {expected}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn second_hello(line: &str) -> String {
    let port = crate::with_live(|live| live.port);
    let mut stream = connect(port);
    write!(stream, "{line}\n").unwrap_or_else(|err| panic!("write: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    read_line(&mut stream)
}

pub fn free_port() -> u16 {
    loop {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").unwrap_or_else(|err| panic!("bind: {err}"));
        let port = listener
            .local_addr()
            .unwrap_or_else(|err| panic!("local_addr: {err}"))
            .port();
        drop(listener);
        if port != 9737 {
            return port;
        }
    }
}

fn wait_listen(port: u16, server: &mut Proc) {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    loop {
        if TcpStream::connect(&addr).is_ok() {
            return;
        }
        if let Some(status) = server
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("try_wait: {err}"))
        {
            panic!(
                "spook run exited {:?}\n{}",
                status.code(),
                support::text_of(&server.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(10) {
            panic!(
                "listener did not open\n{}",
                support::text_of(&server.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn connect(port: u16) -> TcpStream {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    loop {
        match TcpStream::connect(&addr) {
            Ok(stream) => {
                stream
                    .set_nodelay(true)
                    .unwrap_or_else(|err| panic!("nodelay: {err}"));
                return stream;
            }
            Err(err) => {
                if start.elapsed() > Duration::from_secs(5) {
                    panic!("connect {addr}: {err}");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

fn connect_player(port: u16) -> (TcpStream, Arc<Mutex<String>>) {
    let mut stream = connect(port);
    let text = spawn_reader(&stream);
    writeln!(stream, "player:pc_rowan").unwrap_or_else(|err| panic!("hello: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    wait_text(&text, "\"kind\":\"perceive\"");
    (stream, text)
}

fn connect_gm(port: u16) -> (TcpStream, Arc<Mutex<String>>) {
    let mut stream = connect(port);
    let text = spawn_reader(&stream);
    stream
        .write_all(b"gm\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"proof\",\"version\":\"0\"}}}\n")
        .unwrap_or_else(|err| panic!("gm hello: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    wait_text(&text, "\"id\":1");
    writeln!(
        stream,
        "{{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}"
    )
    .unwrap_or_else(|err| panic!("initialized: {err}"));
    stream.flush().unwrap_or_else(|err| panic!("flush: {err}"));
    (stream, text)
}

fn spawn_reader(stream: &TcpStream) -> Arc<Mutex<String>> {
    let read = stream
        .try_clone()
        .unwrap_or_else(|err| panic!("clone: {err}"));
    let text = Arc::new(Mutex::new(String::new()));
    let slot = Arc::clone(&text);
    std::thread::spawn(move || {
        let mut read = read;
        let mut buf = [0u8; 256];
        loop {
            match read.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => slot
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
    });
    text
}

fn wait_text(slot: &Arc<Mutex<String>>, needle: &str) {
    let _ = wait_suffix(slot, 0, needle);
}

fn wait_suffix(slot: &Arc<Mutex<String>>, mark: usize, needle: &str) -> String {
    let start = Instant::now();
    loop {
        let text = support::text_of(slot);
        let added = text.get(mark..).unwrap_or("");
        if added.contains(needle) {
            return added.to_string();
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("missing {needle}\n{added}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn read_line(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => panic!("eof {}", String::from_utf8_lossy(&buf)),
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(err) => panic!("read {err}; {}", String::from_utf8_lossy(&buf)),
        }
    }
    String::from_utf8(buf).unwrap_or_else(|err| panic!("utf-8: {err}"))
}

pub fn run(world: &mut World, args: &[&str]) {
    let output = support::spook(args);
    world.code = output.status.code();
    world.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    world.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
}
