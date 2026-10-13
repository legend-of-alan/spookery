use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;

use crate::protocol::PlayerMessage;
use spookery_kernel::Reason;

use super::render;
use super::TerminalError;

pub fn run_player_terminal(
    pack: &Path,
    as_player: Option<&str>,
    addr: &str,
    raw: bool,
) -> Result<(), TerminalError> {
    let actor = match as_player {
        Some(token) => parse_player_as(token)?,
        None => start_pc(pack)?,
    };
    let mut stream = TcpStream::connect(addr).map_err(TerminalError::Io)?;
    stream.set_nodelay(true).map_err(TerminalError::Io)?;
    stream
        .write_all(crate::protocol::write_player_hello(&actor).as_bytes())
        .map_err(TerminalError::Io)?;
    stream.flush().map_err(TerminalError::Io)?;
    let mut reader = BufReader::new(stream);
    loop {
        let Some(line) = read_line(&mut reader)? else {
            eprintln!("{}", Reason::NotInSpec);
            std::process::exit(1);
        };
        match write_frame(&line, raw) {
            FrameEffect::Refuse => std::process::exit(1),
            FrameEffect::Perceive => break,
            FrameEffect::Continue => {}
        }
    }
    let mut write_stream = reader.get_mut().try_clone().map_err(TerminalError::Io)?;
    std::thread::spawn(move || loop {
        match read_line(&mut reader) {
            Ok(Some(line)) => match write_frame(&line, raw) {
                FrameEffect::Refuse | FrameEffect::Continue | FrameEffect::Perceive => {}
            },
            Ok(None) => std::process::exit(0),
            Err(TerminalError::Io(err)) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
            Err(TerminalError::Reason(code)) => {
                eprintln!("{code}");
                std::process::exit(1);
            }
        }
    });
    for input in io::stdin().lines() {
        let input = input.map_err(TerminalError::Io)?;
        writeln!(write_stream, "{input}").map_err(TerminalError::Io)?;
        write_stream.flush().map_err(TerminalError::Io)?;
    }
    Ok(())
}

fn parse_player_as(token: &str) -> Result<String, TerminalError> {
    let Some(id) = token.strip_prefix("player:") else {
        return Err(TerminalError::Reason(Reason::NotInSpec));
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(TerminalError::Reason(Reason::Schema));
    }
    Ok(id.to_string())
}

fn start_pc(pack: &Path) -> Result<String, TerminalError> {
    let id = match spookery_kernel::canon::load_canon(pack) {
        Ok(canon) => canon.start_pc,
        Err(errors) => match spookery_kernel::first_reason(&errors) {
            Reason::NotInSpec => pack_pc_line(pack)?,
            code => return Err(TerminalError::Reason(code)),
        },
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(TerminalError::Reason(Reason::Schema));
    }
    Ok(id)
}

fn pack_pc_line(pack: &Path) -> Result<String, TerminalError> {
    let text = fs::read_to_string(pack.join("pack.yaml"))
        .map_err(|_| TerminalError::Reason(Reason::NotInSpec))?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("pc:") else {
            continue;
        };
        let mut id = rest.trim();
        if id.len() >= 2 && id.starts_with('"') && id.ends_with('"') {
            id = &id[1..id.len() - 1];
        }
        return Ok(id.to_string());
    }
    Err(TerminalError::Reason(Reason::Schema))
}

fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, TerminalError> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(TerminalError::Io)?;
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
    Perceive,
    Refuse,
}

fn write_frame(line: &str, raw: bool) -> FrameEffect {
    if raw {
        return write_raw(line);
    }
    write_readable(line)
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
        FrameEffect::Perceive
    } else {
        FrameEffect::Continue
    }
}

fn write_readable(line: &str) -> FrameEffect {
    let Ok(message) = crate::protocol::read_player(line) else {
        eprintln!("{line}");
        let _ = io::stderr().flush();
        return FrameEffect::Continue;
    };
    let effect = frame_effect(&message);
    let text = render::render(&message);
    match effect {
        FrameEffect::Refuse => {
            eprintln!("{text}");
            let _ = io::stderr().flush();
        }
        FrameEffect::Continue | FrameEffect::Perceive => {
            println!("{text}");
            let _ = io::stdout().flush();
        }
    }
    effect
}

fn frame_effect(message: &PlayerMessage) -> FrameEffect {
    match message {
        PlayerMessage::Refused { .. } => FrameEffect::Refuse,
        PlayerMessage::Perceive { .. } => FrameEffect::Perceive,
        PlayerMessage::Say { .. }
        | PlayerMessage::Queued { .. }
        | PlayerMessage::Check { .. }
        | PlayerMessage::Resume { .. } => FrameEffect::Continue,
    }
}

fn refused_code(line: &str) -> Option<String> {
    match crate::protocol::read_player(line) {
        Ok(PlayerMessage::Refused { code }) => Some(code),
        _ => None,
    }
}

fn is_perceive(line: &str) -> bool {
    matches!(
        crate::protocol::read_player(line),
        Ok(PlayerMessage::Perceive { .. })
    )
}
