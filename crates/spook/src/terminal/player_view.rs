//! The human player reads a socket and prints each frame.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;

use crate::protocol::PlayerMessage;

use super::render;
use super::Error;

pub fn run_player(
    _pack: &Path,
    as_player: Option<&str>,
    addr: SocketAddr,
    raw: bool,
) -> Result<(), Error> {
    let mut formatter = render::PlayerFormatter::new();
    let actor = match as_player {
        Some(token) => parse_player_as(token)?,
        None => {
            let mut probe = TcpStream::connect(addr).map_err(Error::Io)?;
            probe.set_nodelay(true).map_err(Error::Io)?;
            probe.write_all(b"\n").map_err(Error::Io)?;
            probe.flush().map_err(Error::Io)?;
            let Some(line) = read_line(&mut BufReader::new(probe))? else {
                return Err(Error::Reason("NOT_IN_SPEC"));
            };
            match crate::protocol::read_player(&line) {
                Ok(PlayerMessage::Refused { code, missing }) if code == "CHOOSE_PC" => {
                    let pcs: Vec<&str> = missing.as_deref().unwrap_or("").split(", ").filter(|s| !s.is_empty()).collect();
                    if pcs.len() == 1 {
                        pcs[0].to_string()
                    } else {
                        println!("{}", crate::protocol::named_refusal(&code, missing.as_deref()));
                        let _ = io::stdout().flush();
                        let Some(chosen) = io::stdin().lines().next() else { return Ok(()); };
                        let chosen = chosen.map_err(Error::Io)?;
                        crate::protocol::read_player_hello(chosen.trim()).unwrap_or(chosen.trim()).to_string()
                    }
                }
                _ => {
                    write_frame(&mut formatter, &line, raw);
                    std::process::exit(1);
                }
            }
        }
    };
    let mut stream = TcpStream::connect(addr).map_err(Error::Io)?;
    stream.set_nodelay(true).map_err(Error::Io)?;
    stream
        .write_all(crate::protocol::write_player_hello(&actor).as_bytes())
        .map_err(Error::Io)?;
    stream.flush().map_err(Error::Io)?;
    let mut reader = BufReader::new(stream);
    loop {
        let Some(line) = read_line(&mut reader)? else {
            eprintln!("NOT_IN_SPEC");
            std::process::exit(1);
        };
        match write_frame(&mut formatter, &line, raw) {
            FrameEffect::Refuse => std::process::exit(1),
            FrameEffect::View => break,
            FrameEffect::Continue => {}
        }
    }
    let mut write_stream = reader.get_mut().try_clone().map_err(Error::Io)?;
    std::thread::spawn(move || loop {
        match read_line(&mut reader) {
            Ok(Some(line)) => match write_frame(&mut formatter, &line, raw) {
                FrameEffect::Refuse | FrameEffect::Continue | FrameEffect::View => {}
            },
            Ok(None) => std::process::exit(0),
            Err(Error::Io(err)) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
            Err(Error::Reason(code)) => {
                eprintln!("{code}");
                std::process::exit(1);
            }
        }
    });
    for input in io::stdin().lines() {
        let input = input.map_err(Error::Io)?;
        writeln!(write_stream, "{input}").map_err(Error::Io)?;
        write_stream.flush().map_err(Error::Io)?;
    }
    Ok(())
}

fn parse_player_as(token: &str) -> Result<String, Error> {
    if !token.starts_with("player:") {
        return Err(Error::Reason("NOT_IN_SPEC"));
    }
    crate::protocol::read_player_hello(token)
        .map(str::to_string)
        .ok_or(Error::Reason("SCHEMA"))
}

fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, Error> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(Error::Io)?;
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

enum FrameEffect {
    Continue,
    View,
    Refuse,
}

fn write_frame(formatter: &mut render::PlayerFormatter, line: &str, raw: bool) -> FrameEffect {
    if raw {
        return write_raw(line);
    }
    write_readable(formatter, line)
}

fn write_raw(line: &str) -> FrameEffect {
    if let Some(code) = refused_code(line) {
        eprintln!("{code}");
        let _ = io::stderr().flush();
        return FrameEffect::Refuse;
    }
    println!("{line}");
    let _ = io::stdout().flush();
    if is_perceive(line) {
        FrameEffect::View
    } else {
        FrameEffect::Continue
    }
}

fn write_readable(formatter: &mut render::PlayerFormatter, line: &str) -> FrameEffect {
    let Ok(message) = crate::protocol::read_player(line) else {
        eprintln!("{line}");
        let _ = io::stderr().flush();
        return FrameEffect::Continue;
    };
    let effect = frame_effect(&message);
    let shown = formatter.show(&message);
    if shown.is_empty() {
        return effect;
    }
    match effect {
        FrameEffect::Refuse => {
            eprintln!("{shown}");
            let _ = io::stderr().flush();
        }
        FrameEffect::Continue | FrameEffect::View => {
            println!("{shown}");
            let _ = io::stdout().flush();
        }
    }
    effect
}

fn frame_effect(message: &PlayerMessage) -> FrameEffect {
    match message {
        PlayerMessage::Refused { .. } => FrameEffect::Refuse,
        PlayerMessage::View { .. } => FrameEffect::View,
        PlayerMessage::Say { .. }
        | PlayerMessage::Queued { .. }
        | PlayerMessage::Check { .. }
        | PlayerMessage::Resume { .. }
        | PlayerMessage::Lines { .. } => FrameEffect::Continue,
    }
}

fn refused_code(line: &str) -> Option<String> {
    match crate::protocol::read_player(line) {
        Ok(PlayerMessage::Refused { code, .. }) => Some(code),
        _ => None,
    }
}

fn is_perceive(line: &str) -> bool {
    matches!(
        crate::protocol::read_player(line),
        Ok(PlayerMessage::View { .. })
    )
}
