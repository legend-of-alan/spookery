use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use spookery_kernel::Reason;

use spookery::Error;

pub fn gm(args: &[String]) -> Result<(), Error> {
    if args.len() != 2 || args[1] != "--stdio" {
        return Err(Error::Reason(Reason::NotInSpec));
    }
    let mut stream = TcpStream::connect("127.0.0.1:9737").map_err(Error::Io)?;
    stream.set_nodelay(true).map_err(Error::Io)?;
    stream.write_all(b"gm\n").map_err(Error::Io)?;
    stream.flush().map_err(Error::Io)?;
    bridge_stdio(stream)
}

enum BridgeEnd {
    Local,
    Peer,
    Refused(Vec<u8>),
}

/// Copy bytes. The first socket line, if it is a refused hello, goes to stderr.
fn bridge_stdio(stream: TcpStream) -> Result<(), Error> {
    let local_close = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&local_close);
    let outbound = stream.try_clone().map_err(Error::Io)?;
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
        Err(err) => Err(Error::Io(err)),
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
                if socket
                    .write_all(&buf[..n])
                    .and_then(|_| socket.flush())
                    .is_err()
                {
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

fn refused_code(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("kind").and_then(|kind| kind.as_str()) != Some("refused") {
        return None;
    }
    value
        .get("code")
        .and_then(|code| code.as_str())
        .map(str::to_string)
}
