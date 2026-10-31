//! The human player reads a socket and prints each frame.

use std::io::{self, Write};
use std::net::SocketAddr;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::protocol::{Missing, PlayerFrame};

use super::frame_text;
use super::Error;

pub async fn run_player(as_player: Option<&str>, addr: SocketAddr, raw: bool) -> Result<(), Error> {
    let mut formatter = frame_text::PlayerFormatter::new();
    let mut input = super::stdin_lines();
    let actor = match as_player {
        Some(token) => parse_player_id(token)?,
        None => {
            let probe = dial(addr, b"\n").await?;
            let Some(line) = BufReader::new(probe)
                .lines()
                .next_line()
                .await
                .map_err(Error::Io)?
            else {
                return Err(Error::Message("NOT_IN_SPEC".into()));
            };
            match crate::protocol::PlayerFrame::from_line(&line) {
                Ok(PlayerFrame::Refused {
                    code,
                    detail: missing,
                }) if code == "CHOOSE_PC" => {
                    let pcs = match missing {
                        Some(Missing::Many(pcs)) => pcs,
                        _ => Vec::new(),
                    };
                    match pcs.as_slice() {
                        [] => return Err(Error::Message("No free characters".into())),
                        [only] => only.clone(),
                        many => {
                            println!("Choose a character:\n{}", many.join("\n"));
                            let _ = io::stdout().flush();
                            let Some(chosen) = input.recv().await else {
                                return Ok(());
                            };
                            parse_player_id(&chosen)?
                        }
                    }
                }
                _ => {
                    let reason = match show_frame(&mut formatter, &line, raw) {
                        FrameFlow::Refused(shown) => shown,
                        FrameFlow::Continue | FrameFlow::View => "NOT_IN_SPEC".into(),
                    };
                    return Err(Error::Message(reason));
                }
            }
        }
    };
    let hello = crate::protocol::hello_line(&actor);
    let (reader, mut writer) = dial(addr, hello.as_bytes()).await?.into_split();
    let mut frames = BufReader::new(reader).lines();
    loop {
        let Some(line) = frames.next_line().await.map_err(Error::Io)? else {
            return Err(Error::Message("NOT_IN_SPEC".into()));
        };
        match show_frame(&mut formatter, &line, raw) {
            FrameFlow::Refused(shown) => return Err(Error::Message(shown)),
            FrameFlow::View => break,
            FrameFlow::Continue => {}
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
                if let FrameFlow::Refused(shown) = show_frame(&mut formatter, &line, raw) {
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

fn parse_player_id(token: &str) -> Result<String, Error> {
    let token = token.trim();
    let id = token.strip_prefix("player:").unwrap_or(token);
    crate::protocol::parse_hello(&format!("player:{id}"))
        .map(str::to_string)
        .ok_or(Error::Message("SCHEMA".into()))
}

enum FrameFlow {
    Continue,
    View,
    /// The host refused. Carries the line to show on stderr.
    Refused(String),
}

fn show_frame(formatter: &mut frame_text::PlayerFormatter, line: &str, raw: bool) -> FrameFlow {
    if raw {
        return show_raw(line);
    }
    show_readable(formatter, line)
}

fn show_raw(line: &str) -> FrameFlow {
    if let Some(code) = refused_code(line) {
        return FrameFlow::Refused(code);
    }
    println!("{line}");
    let _ = io::stdout().flush();
    if is_view(line) {
        FrameFlow::View
    } else {
        FrameFlow::Continue
    }
}

fn show_readable(formatter: &mut frame_text::PlayerFormatter, line: &str) -> FrameFlow {
    let Ok(message) = crate::protocol::PlayerFrame::from_line(line) else {
        eprintln!("{line}");
        let _ = io::stderr().flush();
        return FrameFlow::Continue;
    };
    let shown = formatter.format(&message);
    let effect = match message {
        PlayerFrame::Refused { .. } => return FrameFlow::Refused(shown),
        PlayerFrame::View { .. } => FrameFlow::View,
        PlayerFrame::Say { .. }
        | PlayerFrame::Queued { .. }
        | PlayerFrame::Check { .. }
        | PlayerFrame::Resume { .. }
        | PlayerFrame::Lines { .. } => FrameFlow::Continue,
    };
    if !shown.is_empty() {
        println!("{shown}");
        let _ = io::stdout().flush();
    }
    effect
}

fn refused_code(line: &str) -> Option<String> {
    match crate::protocol::PlayerFrame::from_line(line) {
        Ok(PlayerFrame::Refused { code, .. }) => Some(code),
        _ => None,
    }
}

fn is_view(line: &str) -> bool {
    matches!(
        crate::protocol::PlayerFrame::from_line(line),
        Ok(PlayerFrame::View { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::parse_player_id;

    #[test]
    fn a_bare_id_and_a_player_prefix_are_one_id() {
        let bare = parse_player_id("pc_rowan").ok();
        assert_eq!(bare.as_deref(), Some("pc_rowan"));
        assert_eq!(parse_player_id("player:pc_rowan").ok(), bare);
        assert_eq!(parse_player_id("  player:pc_rowan").ok(), bare);
        assert!(parse_player_id("player:").is_err());
        assert!(parse_player_id("").is_err());
    }
}
