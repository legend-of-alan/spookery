//! The human player reads a socket and prints each frame.

use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;

use crate::protocol::PlayerMessage;
use spookery_kernel::api::Reason;

use super::render;
use super::Error;

pub fn run_player(
    pack: &Path,
    as_player: Option<&str>,
    addr: &str,
    raw: bool,
) -> Result<(), Error> {
    let actor = match as_player {
        Some(token) => parse_player_as(token)?,
        None => spookery_kernel::api::canon::load_start_pc(pack).map_err(Error::Reason)?,
    };
    let mut stream = TcpStream::connect(addr).map_err(Error::Io)?;
    stream.set_nodelay(true).map_err(Error::Io)?;
    stream
        .write_all(crate::protocol::write_player_hello(&actor).as_bytes())
        .map_err(Error::Io)?;
    stream.flush().map_err(Error::Io)?;
    let mut reader = BufReader::new(stream);
    let mut formatter = render::PlayerFormatter::new();
    loop {
        let Some(line) = read_line(&mut reader)? else {
            eprintln!("{}", Reason::NotInSpec);
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
    let Some(id) = token.strip_prefix("player:") else {
        return Err(Error::Reason(Reason::NotInSpec));
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(Error::Reason(Reason::Schema));
    }
    Ok(id.to_string())
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
