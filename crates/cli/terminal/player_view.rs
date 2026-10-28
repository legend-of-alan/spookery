//! The human player reads a socket and prints each frame.

use std::io::{self, Write};
use std::net::SocketAddr;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::protocol::{Missing, PlayerMessage};

use super::render;
use super::Error;

pub async fn run_player(as_player: Option<&str>, addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let mut formatter = render::PlayerFormatter::new();
    let mut input = super::stdin_lines();
    let actor = match as_player {
        Some(token) => player_id(token)?,
        None => {
            let probe = dial(addr, b"\n").await?;
            let Some(line) = BufReader::new(probe)
                .lines()
                .next_line()
                .await
                .map_err(Error::Io)?
            else {
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
                            let Some(chosen) = input.recv().await else {
                                return Ok(());
                            };
                            player_id(&chosen)?
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
    let hello = crate::protocol::write_player_hello(&actor);
    let (reader, mut writer) = dial(addr, hello.as_bytes()).await?.into_split();
    let mut frames = BufReader::new(reader).lines();
    loop {
        let Some(line) = frames.next_line().await.map_err(Error::Io)? else {
            return Err(Error::Reason("NOT_IN_SPEC".into()));
        };
        match write_frame(&mut formatter, &line, raw) {
            FrameEffect::Refuse(shown) => return Err(Error::Reason(shown)),
            FrameEffect::View => break,
            FrameEffect::Continue => {}
        }
    }
    loop {
        tokio::select! {
            // `next_line` is cancel safe, so a frame cut by a typed line is kept.
            line = frames.next_line() => {
                let Some(line) = line.map_err(Error::Io)? else {
                    eprintln!("host closed");
                    return Ok(());
                };
                if let FrameEffect::Refuse(shown) = write_frame(&mut formatter, &line, raw) {
                    eprintln!("{shown}");
                    let _ = io::stderr().flush();
                }
            }
            typed = input.recv() => {
                let Some(typed) = typed else {
                    return Ok(());
                };
                writer
                    .write_all(format!("{typed}\n").as_bytes())
                    .await
                    .map_err(Error::Io)?;
            }
        }
    }
}

/// Open the game port and send one hello line.
async fn dial(addr: SocketAddr, hello: &[u8]) -> Result<TcpStream, Error> {
    let mut stream = TcpStream::connect(addr).await.map_err(Error::Io)?;
    stream.set_nodelay(true).map_err(Error::Io)?;
    stream.write_all(hello).await.map_err(Error::Io)?;
    Ok(stream)
}

fn player_id(token: &str) -> Result<String, Error> {
    let token = token.trim();
    let id = token.strip_prefix("player:").unwrap_or(token);
    crate::protocol::read_player_hello(&format!("player:{id}"))
        .map(str::to_string)
        .ok_or(Error::Reason("SCHEMA".into()))
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
