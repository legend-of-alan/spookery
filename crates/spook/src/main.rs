//! The `spook` command for one loopback table.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use spookery_kernel::api::Reason;

mod starter;

/// One loopback table.
#[derive(Parser)]
#[command(name = "spook", version)]
struct Cli {
    /// Loopback address. `127.0.0.1` means port 9737. On `run`, port 0 lets the OS choose.
    #[arg(long, global = true, default_value = "127.0.0.1:9737")]
    addr: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Listen for the player seat and the GM seat.
    Run {
        /// Pack directory.
        pack: PathBuf,
        /// Agent door. Defaults to the next port. Port 0 lets the OS choose.
        #[arg(long = "mcp-http", value_name = "127.0.0.1:<port>")]
        mcp_http: Option<String>,
    },
    /// Sit the player seat.
    Play {
        /// Pack directory.
        pack: PathBuf,
        /// Actor to sit, as `player:<id>`.
        #[arg(long = "as", value_name = "player:<id>")]
        as_player: Option<String>,
        /// Print each protocol frame as one JSON line.
        #[arg(long)]
        raw: bool,
    },
    /// Sit the GM seat.
    Gm {
        /// Pack directory.
        pack: PathBuf,
        /// Agent door. Defaults to the game port plus one.
        #[arg(long = "mcp-http", value_name = "127.0.0.1:<port>")]
        mcp_http: Option<String>,
        /// Print each protocol frame as one JSON line.
        #[arg(long)]
        raw: bool,
    },
    /// Sit the GM seat and call the model.
    #[command(name = "ai-gm")]
    AiGm {
        /// Pack directory.
        pack: PathBuf,
        /// Agent door. Defaults to the game port plus one.
        #[arg(long = "mcp-http", value_name = "127.0.0.1:<port>")]
        mcp_http: Option<String>,
    },
    /// Advance the pack one tick.
    Tick {
        /// Pack directory.
        pack: PathBuf,
    },
    /// Print the host digest.
    Status {
        /// Pack directory.
        pack: PathBuf,
    },
    /// Store the current canon hash in the save.
    Migrate {
        /// Pack directory.
        pack: PathBuf,
    },
    /// Replay the journal and byte-compare `save.yaml`.
    Verify {
        /// Pack directory.
        pack: PathBuf,
        /// Directory whose `state/save.yaml` is the replay base.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Check the chain and replay only the latest bundle.
        #[arg(long)]
        quick: bool,
    },
    /// Report pack problems and write nothing.
    Check {
        /// Print problems as one JSON array on stdout.
        #[arg(long)]
        json: bool,
        /// Print each rule id in the resolved order.
        #[arg(long)]
        rules: bool,
        /// Pack directory.
        pack: PathBuf,
    },
    /// Write a minimal pack and print the next commands.
    New {
        /// Directory to create. An existing non-empty directory is refused.
        #[arg(value_name = "DIR")]
        dir: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(exit) = dispatch(cli) {
        match exit {
            spookery::Error::Reason(code) => eprintln!("{code}"),
            spookery::Error::Io(err) => eprintln!("{err}"),
        }
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> Result<(), spookery::Error> {
    // `--addr` is global: every command refuses an open host before any pack opens.
    let addr = parse_loopback(&cli.addr)?;
    match cli.command {
        Command::Run { pack, mcp_http } => {
            let http = match mcp_http {
                Some(text) => Some(parse_loopback(&text)?),
                None => None,
            };
            spookery::host::listen(&pack, addr, http)
        }
        Command::Play {
            pack,
            as_player,
            raw,
        } => from_terminal(spookery::terminal::run_player_terminal(
            &pack,
            as_player.as_deref(),
            &addr.to_string(),
            raw,
        )),
        Command::Gm {
            pack,
            mcp_http,
            raw,
        } => {
            let door = door_addr(mcp_http)?;
            from_terminal(spookery::terminal::run_gm_terminal(
                &pack,
                &addr.to_string(),
                door.as_deref(),
                raw,
            ))
        }
        Command::AiGm { pack, mcp_http } => {
            let door = door_addr(mcp_http)?;
            if spookery::ai::sit(&pack, &addr.to_string(), door.as_deref()).is_err() {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Tick { pack } => {
            if let Some(warning) =
                spookery_round::play_pack_once(&pack).map_err(spookery::Error::Reason)?
            {
                eprintln!("{warning}");
            }
            Ok(())
        }
        Command::Status { pack } => {
            let world = spookery_round::resume(&pack).map_err(spookery::Error::Reason)?;
            let text =
                spookery_kernel::api::host_summary(&world).map_err(spookery::Error::Reason)?;
            print!("{text}");
            Ok(())
        }
        Command::Migrate { pack } => {
            spookery_round::migrate_pack(&pack).map_err(spookery::Error::Reason)
        }
        Command::Verify { pack, from, quick } => {
            if quick && from.is_some() {
                eprintln!("verify --quick does not take --from");
                std::process::exit(1);
            }
            let checked = if quick {
                spookery_round::verify_quick(&pack)
            } else if from.is_none() {
                spookery_round::verify_parallel(&pack)
            } else {
                spookery_round::verify_pack(&pack, from.as_deref())
            };
            match checked {
                Ok(report) => {
                    println!(
                        "verified {} records, state_rev {}",
                        report.applied, report.state_rev
                    );
                    Ok(())
                }
                Err(err) => {
                    eprintln!("{err}");
                    std::process::exit(1);
                }
            }
        }
        Command::Check { pack, json, rules } => {
            let mut found = spookery_kernel::api::check_pack(&pack);
            if rules && found.is_empty() {
                let canon = spookery_kernel::api::canon::load_canon(&pack).map_err(|diags| {
                    spookery::Error::Reason(spookery_kernel::api::reason_of(diags))
                })?;
                for id in canon.rules.ids_in_order() {
                    println!("{id}");
                }
                return Ok(());
            }
            if json {
                found.sort_by(|left, right| {
                    left.file
                        .cmp(&right.file)
                        .then(left.line.cmp(&right.line))
                        .then(left.path.cmp(&right.path))
                });
                let text = serde_json::to_string(&found).expect("diagnostic json");
                println!("{text}");
            } else if found.is_empty() {
                println!("no problems");
            } else {
                for item in &found {
                    eprintln!("{}", item.report_line());
                }
            }
            let errors = found.iter().any(|item| item.code != "WARNING");
            if !errors {
                Ok(())
            } else {
                std::process::exit(1);
            }
        }
        Command::New { dir } => match starter::write_starter(&dir) {
            Ok(next) => {
                print!("{next}");
                Ok(())
            }
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        },
    }
}

fn from_terminal(
    result: Result<(), spookery::terminal::TerminalError>,
) -> Result<(), spookery::Error> {
    result.map_err(|err| match err {
        spookery::terminal::TerminalError::Reason(code) => spookery::Error::Reason(code),
        spookery::terminal::TerminalError::Io(err) => spookery::Error::Io(err),
    })
}

fn parse_loopback(addr: &str) -> Result<SocketAddr, spookery::Error> {
    if addr == "127.0.0.1" {
        return Ok(SocketAddr::from(([127, 0, 0, 1], 9737)));
    }
    let Some(port) = addr.strip_prefix("127.0.0.1:") else {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    };
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    }
    let port: u16 = port
        .parse()
        .map_err(|_| spookery::Error::Reason(Reason::NotInSpec))?;
    Ok(SocketAddr::from(([127, 0, 0, 1], port)))
}

fn door_addr(mcp_http: Option<String>) -> Result<Option<String>, spookery::Error> {
    match mcp_http {
        Some(text) => Ok(Some(parse_loopback(&text)?.to_string())),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_default_address_is_loopback_9737() {
        let cli = Cli::try_parse_from(["spook", "tick", "pack"]).unwrap();
        assert_eq!(cli.addr, "127.0.0.1:9737");
        assert!(matches!(
            parse_loopback("127.0.0.1"),
            Ok(addr) if addr == SocketAddr::from(([127, 0, 0, 1], 9737))
        ));
        assert!(matches!(
            parse_loopback(&cli.addr),
            Ok(addr) if addr.port() == 9737
        ));
        assert!(matches!(
            parse_loopback("127.0.0.1:0"),
            Ok(addr) if addr.port() == 0
        ));
    }

    #[test]
    fn a_subcommand_address_overrides_the_default() {
        let after =
            Cli::try_parse_from(["spook", "run", "--addr", "127.0.0.1:9800", "pack"]).unwrap();
        assert_eq!(after.addr, "127.0.0.1:9800");
        let before =
            Cli::try_parse_from(["spook", "--addr", "127.0.0.1:9800", "run", "pack"]).unwrap();
        assert_eq!(before.addr, "127.0.0.1:9800");
        assert!(matches!(after.command, Command::Run { .. }));
    }

    #[test]
    fn an_open_address_is_not_loopback() {
        assert!(parse_loopback("0.0.0.0").is_err());
        assert!(parse_loopback("10.0.0.8").is_err());
        assert!(parse_loopback("localhost").is_err());
        assert!(parse_loopback("127.0.0.1:").is_err());
    }

    #[test]
    fn every_command_refuses_an_open_address_before_the_pack_opens() {
        for command in [
            "run", "play", "gm", "ai-gm", "tick", "status", "migrate", "check", "verify", "new",
        ] {
            for addr in ["0.0.0.0", "10.0.0.1:9737"] {
                let cli = Cli::try_parse_from(["spook", command, "--addr", addr, "no-such-pack"])
                    .unwrap();
                assert!(
                    matches!(
                        dispatch(cli),
                        Err(spookery::Error::Reason(Reason::NotInSpec))
                    ),
                    "{command} --addr {addr}"
                );
            }
        }
    }

    #[test]
    fn usage_is_not_an_engine_refusal() {
        assert!(Cli::try_parse_from(["spook", "rnu", "x"]).is_err());
        assert!(Cli::try_parse_from(["spook", "gm"]).is_err());
        let gm = Cli::try_parse_from(["spook", "gm", "pack"]).unwrap();
        assert!(matches!(gm.command, Command::Gm { raw: false, .. }));
        let gm_raw = Cli::try_parse_from(["spook", "gm", "--raw", "pack"]).unwrap();
        assert!(matches!(gm_raw.command, Command::Gm { raw: true, .. }));
        assert!(Cli::try_parse_from(["spook", "run", "--as", "player:rowan"]).is_err());
        assert!(Cli::try_parse_from(["spook"]).is_err());
        assert!(Cli::try_parse_from(["spook", "mcp"]).is_err());
        assert!(Cli::try_parse_from(["spook", "mcp", "--addr", "127.0.0.1:9737"]).is_err());
        let http = Cli::try_parse_from(["spook", "run", "--mcp-http", "0.0.0.0:9", "no-such-pack"])
            .unwrap();
        assert!(matches!(
            dispatch(http),
            Err(spookery::Error::Reason(Reason::NotInSpec))
        ));
        let http_loopback =
            Cli::try_parse_from(["spook", "run", "--mcp-http", "127.0.0.1:9801", "pack"]).unwrap();
        assert!(matches!(
            http_loopback.command,
            Command::Run {
                mcp_http: Some(text),
                ..
            } if text == "127.0.0.1:9801"
        ));
        for command in ["gm", "ai-gm"] {
            let open =
                Cli::try_parse_from(["spook", command, "--mcp-http", "0.0.0.0:9", "no-such-pack"])
                    .unwrap();
            assert!(
                matches!(
                    dispatch(open),
                    Err(spookery::Error::Reason(Reason::NotInSpec))
                ),
                "{command} --mcp-http"
            );
            let door =
                Cli::try_parse_from(["spook", command, "--mcp-http", "127.0.0.1:9801", "pack"])
                    .unwrap();
            let text = match door.command {
                Command::Gm { mcp_http, .. } | Command::AiGm { mcp_http, .. } => mcp_http,
                _ => panic!("{command}"),
            };
            assert_eq!(text.as_deref(), Some("127.0.0.1:9801"));
        }
        let play = Cli::try_parse_from(["spook", "play", "pack", "--as", "player:rowan"]).unwrap();
        match play.command {
            Command::Play {
                as_player,
                pack,
                raw,
            } => {
                assert_eq!(as_player.as_deref(), Some("player:rowan"));
                assert_eq!(pack, Path::new("pack"));
                assert!(!raw);
            }
            _ => panic!("play"),
        }
        let raw = Cli::try_parse_from(["spook", "play", "--raw", "pack"]).unwrap();
        assert!(matches!(raw.command, Command::Play { raw: true, .. }));
        let host = Cli::try_parse_from(["spook", "ai-gm", "pack"]).unwrap();
        assert!(matches!(host.command, Command::AiGm { .. }));
        let verify = Cli::try_parse_from(["spook", "verify", "--from", "snap", "pack"]).unwrap();
        assert!(matches!(
            verify.command,
            Command::Verify {
                from: Some(path),
                quick: false,
                ..
            } if path == Path::new("snap")
        ));
        let quick = Cli::try_parse_from(["spook", "verify", "--quick", "pack"]).unwrap();
        assert!(matches!(quick.command, Command::Verify { quick: true, .. }));
    }
}
