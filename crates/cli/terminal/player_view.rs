//! The human player reads a socket and prints each frame.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};

use crate::protocol::{Missing, PlayerMessage};

use super::render;
use super::Error;

pub fn run_player(as_player: Option<&str>, addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let mut formatter = render::PlayerFormatter::new();
    let actor = match as_player {
        Some(token) => player_id(token)?,
        None => {
            let mut probe = TcpStream::connect(addr).map_err(Error::Io)?;
            probe.set_nodelay(true).map_err(Error::Io)?;
            probe.write_all(b"\n").map_err(Error::Io)?;
            probe.flush().map_err(Error::Io)?;
            let Some(line) = read_line(&mut BufReader::new(probe))? else {
                return Err(Error::Reason("NOT_IN_SPEC".into()));
            };
            match crate::protocol::read_player(&line) {
                Ok(PlayerMessage::Refused { code, missing }) if code == "CHOOSE_PC" => {
                    let pcs = match missing {
                        Some(Missing::Many(pcs)) => pcs,
                        _ => Vec::new(),
                    };
                    match pcs.as_slice() {
                        [] => return Err(Error::Reason("No free characters".into())),
                        [only] => only.clone(),
                        many => {
                            println!("Choose a character:\n{}", many.join("\n"));
                            let _ = io::stdout().flush();
                            let Some(chosen) = io::stdin().lines().next() else {
                                return Ok(());
                            };
                            player_id(&chosen.map_err(Error::Io)?)?
                        }
                    }
                }
                _ => {
                    let reason = match write_frame(&mut formatter, &line, raw) {
                        FrameEffect::Refuse(shown) => shown,
                        FrameEffect::Continue | FrameEffect::View => "NOT_IN_SPEC".into(),
                    };
                    return Err(Error::Reason(reason));
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
            return Err(Error::Reason("NOT_IN_SPEC".into()));
        };
        match write_frame(&mut formatter, &line, raw) {
            FrameEffect::Refuse(shown) => return Err(Error::Reason(shown)),
            FrameEffect::View => break,
            FrameEffect::Continue => {}
        }
    }
    let mut write_stream = reader.get_mut().try_clone().map_err(Error::Io)?;
    std::thread::spawn(move || loop {
        match read_line(&mut reader) {
            Ok(Some(line)) => match write_frame(&mut formatter, &line, raw) {
                FrameEffect::Refuse(shown) => {
                    eprintln!("{shown}");
                    let _ = io::stderr().flush();
                }
                FrameEffect::Continue | FrameEffect::View => {}
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

fn player_id(token: &str) -> Result<String, Error> {
    let token = token.trim();
    let id = token.strip_prefix("player:").unwrap_or(token);
    crate::protocol::read_player_hello(&format!("player:{id}"))
        .map(str::to_string)
        .ok_or(Error::Reason("SCHEMA".into()))
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
    /// The host refused. Carries the line to show on stderr.
    Refuse(String),
}

fn write_frame(formatter: &mut render::PlayerFormatter, line: &str, raw: bool) -> FrameEffect {
    if raw {
        return write_raw(line);
    }
    write_readable(formatter, line)
}

fn write_raw(line: &str) -> FrameEffect {
    if let Some(code) = refused_code(line) {
        return FrameEffect::Refuse(code);
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
    let shown = formatter.show(&message);
    let effect = match message {
        PlayerMessage::Refused { .. } => return FrameEffect::Refuse(shown),
        PlayerMessage::View { .. } => FrameEffect::View,
        PlayerMessage::Say { .. }
        | PlayerMessage::Queued { .. }
        | PlayerMessage::Check { .. }
        | PlayerMessage::Resume { .. }
        | PlayerMessage::Lines { .. } => FrameEffect::Continue,
    };
    if !shown.is_empty() {
        println!("{shown}");
        let _ = io::stdout().flush();
    }
    effect
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

#[cfg(test)]
mod tests {
    use super::player_id;

    #[test]
    fn a_bare_id_and_a_player_prefix_are_one_id() {
        let bare = player_id("pc_rowan").ok();
        assert_eq!(bare.as_deref(), Some("pc_rowan"));
        assert_eq!(player_id("player:pc_rowan").ok(), bare);
        assert_eq!(player_id("  player:pc_rowan").ok(), bare);
        assert!(player_id("player:").is_err());
        assert!(player_id("").is_err());
    }
}
