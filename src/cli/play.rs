use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use spookery_kernel::Code;

use super::Exit;

pub fn play(args: &[String]) -> Result<(), Exit> {
    if args.is_empty() {
        return Err(Exit::Code(Code::NotInSpec));
    }
    let pack = Path::new(&args[0]);
    let actor = if args.len() == 1 {
        start_pc(pack)?
    } else if args.len() == 3 && args[1] == "--as" {
        parse_player_as(&args[2])?
    } else {
        return Err(Exit::Code(Code::NotInSpec));
    };
    let mut stream = TcpStream::connect("127.0.0.1:9737").map_err(Exit::Os)?;
    stream.set_nodelay(true).map_err(Exit::Os)?;
    write!(stream, "player:{actor}\n").map_err(Exit::Os)?;
    stream.flush().map_err(Exit::Os)?;
    let mut reader = BufReader::new(stream);
    loop {
        let Some(line) = read_line(&mut reader)? else {
            eprintln!("{}", Code::NotInSpec);
            std::process::exit(1);
        };
        if let Some(code) = refused_code(&line) {
            eprintln!("{code}");
            std::process::exit(1);
        }
        println!("{line}");
        let _ = io::stdout().flush();
        if line.starts_with("{\"kind\":\"perceive\"") {
            break;
        }
    }
    let mut write_stream = reader.get_mut().try_clone().map_err(Exit::Os)?;
    std::thread::spawn(move || {
        loop {
            match read_line(&mut reader) {
                Ok(Some(line)) => {
                    if let Some(code) = refused_code(&line) {
                        eprintln!("{code}");
                        let _ = io::stderr().flush();
                        continue;
                    }
                    println!("{line}");
                    let _ = io::stdout().flush();
                }
                Ok(None) => std::process::exit(0),
                Err(Exit::Os(err)) => {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
                Err(Exit::Code(code)) => {
                    eprintln!("{code}");
                    std::process::exit(1);
                }
            }
        }
    });
    for input in io::stdin().lines() {
        let input = input.map_err(Exit::Os)?;
        writeln!(write_stream, "{input}").map_err(Exit::Os)?;
        write_stream.flush().map_err(Exit::Os)?;
    }
    Ok(())
}

pub fn gm(args: &[String]) -> Result<(), Exit> {
    if args.is_empty() {
        return Err(Exit::Code(Code::NotInSpec));
    }
    let stdio = if args.len() == 1 {
        false
    } else if args.len() == 2 && args[1] == "--stdio" {
        true
    } else {
        return Err(Exit::Code(Code::NotInSpec));
    };
    let mut stream = TcpStream::connect("127.0.0.1:9737").map_err(Exit::Os)?;
    stream.set_nodelay(true).map_err(Exit::Os)?;
    if stdio {
        stream.write_all(b"gm\n").map_err(Exit::Os)?;
        stream.flush().map_err(Exit::Os)?;
        return bridge_stdio(stream);
    }
    stream.write_all(b"gm\n").map_err(Exit::Os)?;
    stream.flush().map_err(Exit::Os)?;
    let watch = stream.try_clone().map_err(Exit::Os)?;
    std::thread::spawn(move || {
        for line in io::stdin().lines() {
            match line {
                Ok(line) if line == "close" => {
                    let _ = watch.shutdown(std::net::Shutdown::Write);
                    // Dropping this clone resets the socket before the server finishes the seat drop.
                    std::mem::forget(watch);
                    return;
                }
                Ok(_) => eprintln!("{}", Code::NotInSpec),
                Err(_) => {
                    let _ = watch.shutdown(std::net::Shutdown::Both);
                    return;
                }
            }
        }
        let _ = watch.shutdown(std::net::Shutdown::Both);
    });
    let mut reader = BufReader::new(stream);
    loop {
        let Some(line) = read_line(&mut reader)? else {
            return Ok(());
        };
        if let Some(code) = refused_code(&line) {
            eprintln!("{code}");
            std::process::exit(1);
        }
        if let Some(params) = event_params(&line) {
            println!("{params}");
            let _ = io::stdout().flush();
        }
    }
}

enum BridgeEnd {
    Local,
    Peer,
    Refused(Vec<u8>),
}

/// Copy bytes. The first socket line, if it is a refused hello, goes to stderr.
fn bridge_stdio(stream: TcpStream) -> Result<(), Exit> {
    let local_close = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&local_close);
    let outbound = stream.try_clone().map_err(Exit::Os)?;
    std::thread::spawn(move || copy_stdin(outbound, flag));
    match copy_socket(stream, &local_close) {
        Ok(BridgeEnd::Local) => Ok(()),
        Ok(BridgeEnd::Peer) => {
            std::process::exit(1);
        }
        Ok(BridgeEnd::Refused(line)) => {
            let mut err = io::stderr();
            let _ = err.write_all(&line);
            if !line.ends_with(b"\n") {
                let _ = err.write_all(b"\n");
            }
            let _ = err.flush();
            std::process::exit(1);
        }
        Err(err) => Err(Exit::Os(err)),
    }
}

fn copy_stdin(mut socket: TcpStream, local_close: Arc<AtomicBool>) {
    let mut stdin = io::stdin();
    let mut buf = [0u8; 8192];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => {
                local_close.store(true, Ordering::Release);
                let _ = socket.shutdown(Shutdown::Both);
                return;
            }
            Ok(n) => {
                if socket.write_all(&buf[..n]).and_then(|_| socket.flush()).is_err() {
                    let _ = socket.shutdown(Shutdown::Both);
                    return;
                }
            }
            Err(_) => {
                let _ = socket.shutdown(Shutdown::Both);
                return;
            }
        }
    }
}

fn copy_socket(mut socket: TcpStream, local_close: &AtomicBool) -> io::Result<BridgeEnd> {
    let mut incoming = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut passed_hello = false;
    loop {
        let read = match socket.read(&mut chunk) {
            Ok(0) => {
                if local_close.load(Ordering::Acquire) {
                    return Ok(BridgeEnd::Local);
                }
                if !passed_hello {
                    if let Some(end) = incoming.iter().position(|byte| *byte == b'\n') {
                        let line = &incoming[..=end];
                        if is_refused_hello(line) {
                            return Ok(BridgeEnd::Refused(line.to_vec()));
                        }
                    }
                }
                if !incoming.is_empty() {
                    io::stdout().write_all(&incoming)?;
                    io::stdout().flush()?;
                }
                return Ok(BridgeEnd::Peer);
            }
            Ok(n) => n,
            Err(err) => {
                if local_close.load(Ordering::Acquire) {
                    return Ok(BridgeEnd::Local);
                }
                return Err(err);
            }
        };
        incoming.extend_from_slice(&chunk[..read]);
        if !passed_hello {
            let Some(end) = incoming.iter().position(|byte| *byte == b'\n') else {
                continue;
            };
            let line = incoming[..=end].to_vec();
            if is_refused_hello(&line) {
                return Ok(BridgeEnd::Refused(line));
            }
            io::stdout().write_all(&line)?;
            io::stdout().flush()?;
            incoming.drain(..=end);
            passed_hello = true;
        }
        if !incoming.is_empty() {
            io::stdout().write_all(&incoming)?;
            io::stdout().flush()?;
            incoming.clear();
        }
    }
}

fn is_refused_hello(line: &[u8]) -> bool {
    let text = String::from_utf8_lossy(line);
    let text = text.trim_end_matches(['\n', '\r']);
    refused_code(text).is_some()
}

fn parse_player_as(token: &str) -> Result<String, Exit> {
    let Some(id) = token.strip_prefix("player:") else {
        return Err(Exit::Code(Code::NotInSpec));
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(Exit::Code(Code::Schema));
    }
    Ok(id.to_string())
}

fn start_pc(pack: &Path) -> Result<String, Exit> {
    let text = fs::read_to_string(pack.join("pack.yaml")).map_err(|_| Exit::Code(Code::NotInSpec))?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("pc:") else {
            continue;
        };
        let mut id = rest.trim();
        if id.len() >= 2 && id.starts_with('"') && id.ends_with('"') {
            id = &id[1..id.len() - 1];
        }
        if id.is_empty() || id.chars().any(char::is_whitespace) {
            return Err(Exit::Code(Code::Schema));
        }
        return Ok(id.to_string());
    }
    Err(Exit::Code(Code::Schema))
}

fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, Exit> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(Exit::Os)?;
    if read == 0 {
        return Ok(None);
    }
    if line.ends_with('\n') {
        line.pop();
    }
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(Some(line))
}

fn event_params(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("method").and_then(|method| method.as_str()) != Some("spook/event") {
        return None;
    }
    let marker = "\"params\":";
    let start = line.find(marker)? + marker.len();
    let params = line[start..].strip_suffix('}')?;
    if !params.starts_with('{') {
        return None;
    }
    Some(params.to_string())
}

fn refused_code(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("kind").and_then(|kind| kind.as_str()) != Some("refused") {
        return None;
    }
    value.get("code").and_then(|code| code.as_str()).map(str::to_string)
}
