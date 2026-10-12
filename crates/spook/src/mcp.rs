use std::io::{self, Write};
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::Error;

/// Dial the running GM seat and copy bytes between stdio and that socket.
///
/// The seat speaks newline-delimited JSON-RPC after `gm`, the same framing as
/// MCP stdio, so this is a byte relay. A refused hello is a sentence on stderr
/// and nothing on stdout.
pub fn bridge(addr: SocketAddr) -> Result<(), Error> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(Error::Io)?;
    runtime.block_on(relay(addr))
}

async fn relay(addr: SocketAddr) -> Result<(), Error> {
    let mut stream = TcpStream::connect(addr).await.map_err(Error::Io)?;
    stream.set_nodelay(true).map_err(Error::Io)?;
    stream.write_all(b"gm\n").await.map_err(Error::Io)?;
    stream.flush().await.map_err(Error::Io)?;
    let (mut read, mut write) = stream.into_split();
    let to_socket = tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let _ = tokio::io::copy(&mut stdin, &mut write).await;
        let _ = write.shutdown().await;
    });
    let first = match read_line(&mut read).await {
        Ok(Some(line)) => line,
        Ok(None) => exit_closed(),
        Err(err) => return Err(Error::Io(err)),
    };
    if let Some(code) = refused_code(&first) {
        exit_refused(&code);
    }
    let mut stdout = tokio::io::stdout();
    stdout
        .write_all(first.as_bytes())
        .await
        .map_err(Error::Io)?;
    stdout.write_all(b"\n").await.map_err(Error::Io)?;
    let copied = tokio::io::copy(&mut read, &mut stdout).await;
    let _ = stdout.flush().await;
    let _ = to_socket.await;
    copied.map_err(Error::Io)?;
    Ok(())
}

fn exit_closed() -> ! {
    eprintln!("seat closed");
    let _ = io::stderr().flush();
    std::process::exit(1);
}

fn exit_refused(code: &str) -> ! {
    let sentence = match code {
        "SESSION_FULL" => "The table is full.",
        "LINE_TOO_LONG" => "That line is too long.",
        "RATE_LIMITED" => "Slow down.",
        _ => "The table refused that.",
    };
    eprintln!("{sentence} [{code}]");
    let _ = io::stderr().flush();
    std::process::exit(1);
}

fn refused_code(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    if value.get("kind").and_then(serde_json::Value::as_str) != Some("refused") {
        return None;
    }
    value
        .get("code")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

async fn read_line<R: tokio::io::AsyncRead + Unpin>(read: &mut R) -> io::Result<Option<String>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match read.read(&mut byte).await {
            Ok(0) => {
                if buf.is_empty() {
                    return Ok(None);
                }
                break;
            }
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
            }
            Err(err) => return Err(err),
        }
    }
    if buf.last() == Some(&b'\r') {
        buf.pop();
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}
