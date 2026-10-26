//! The `spook` command for one loopback table.

use std::net::SocketAddr;
#[cfg(feature = "server")]
use std::path::PathBuf;

use clap::{Parser, Subcommand};

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
    #[cfg(feature = "server")]
    Run {
        /// Pack directory.
        pack: PathBuf,
        /// Agent door. Defaults to the next port. Port 0 lets the OS choose.
        #[arg(long = "mcp-http", value_name = "127.0.0.1:<port>")]
        mcp_http: Option<String>,
    },
    /// Sit the player seat.
    #[cfg(feature = "client")]
    Play {
        /// Actor to sit. A pack id, or `player:<id>`.
        #[arg(long = "as", value_name = "ID")]
        as_player: Option<String>,
        /// Print each protocol frame as one JSON line.
        #[arg(long)]
        raw: bool,
    },
    /// Sit the GM seat.
    #[cfg(feature = "client")]
    Gm {
        /// Agent door. Defaults to the game port plus one.
        #[arg(long = "mcp-http", value_name = "127.0.0.1:<port>")]
        mcp_http: Option<String>,
        /// Print each protocol frame as one JSON line.
        #[arg(long)]
        raw: bool,
    },
    /// Sit the GM seat and call the model.
    #[cfg(feature = "client")]
    #[command(name = "ai-gm")]
    AiGm {
        /// Agent door. Defaults to the game port plus one.
        #[arg(long = "mcp-http", value_name = "127.0.0.1:<port>")]
        mcp_http: Option<String>,
    },
    /// Advance the pack one tick.
    #[cfg(feature = "server")]
    Tick {
        /// Pack directory.
        pack: PathBuf,
    },
    /// Print the host digest.
    #[cfg(feature = "server")]
    Status {
        /// Pack directory.
        pack: PathBuf,
    },
    /// Store the current canon hash in the save.
    #[cfg(feature = "server")]
    Migrate {
        /// Pack directory.
        pack: PathBuf,
    },
    /// Replay the journal and byte-compare `save.yaml`.
    #[cfg(feature = "server")]
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
    #[cfg(feature = "server")]
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
    #[cfg(feature = "server")]
    New {
        /// Directory to create. An existing non-empty directory is refused.
        #[arg(value_name = "DIR")]
        dir: PathBuf,
    },
}

fn main() {
    let cli = Cli::parse();
    if let Err(exit) = dispatch(cli) {
        eprintln!("{exit}");
        std::process::exit(1);
    }
}

fn dispatch(cli: Cli) -> Result<(), String> {
    // `--addr` is global: every command refuses an open host before any pack opens.
    let addr = parse_loopback(&cli.addr)?;
    match cli.command {
        #[cfg(feature = "server")]
        Command::Run { pack, mcp_http } => {
            let door = door_addr(mcp_http)?;
            let world = spookery_engine::World::resume(&pack).map_err(|e| e.to_string())?;
            let mut config = spookery_server::Config::new(addr);
            config.door = door;
            if let Some(ms) = std::env::var("SPOOK_HELLO_TIMEOUT_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .filter(|&ms| ms > 0)
            {
                config.hello_timeout = std::time::Duration::from_millis(ms);
            }
            config.out = Box::new(std::io::stdout());
            config.err = Box::new(std::io::stderr());
            use std::sync::atomic::{AtomicBool, Ordering};
            // The first Ctrl+C or SIGTERM wakes this thread to stop the host. A second
            // one exits at once.
            static SIGNALED: AtomicBool = AtomicBool::new(false);
            let main = std::thread::current();
            ctrlc::set_handler(move || {
                if SIGNALED.swap(true, Ordering::SeqCst) {
                    std::process::exit(130);
                }
                main.unpark();
            })
            .map_err(|e| e.to_string())?;
            let handle = spookery_server::serve(world, config).map_err(|e| e.to_string())?;
            while !SIGNALED.load(Ordering::SeqCst) {
                std::thread::park();
            }
            handle.stop();
            handle.join().map_err(|e| e.to_string())
        }
        #[cfg(feature = "client")]
        Command::Play { as_player, raw } => from_terminal(spookery::terminal::run_player(
            as_player.as_deref(),
            addr,
            raw,
        )),
        #[cfg(feature = "client")]
        Command::Gm { mcp_http, raw } => {
            from_terminal(spookery::terminal::run_gm(addr, door_addr(mcp_http)?, raw))
        }
        #[cfg(feature = "client")]
        Command::AiGm { mcp_http } => {
            let door = door_addr(mcp_http)?;
            if spookery::ai::sit(addr, door).is_err() {
                std::process::exit(1);
            }
            Ok(())
        }
        #[cfg(feature = "server")]
        Command::Tick { pack } => {
            if let Some(warning) =
                spookery_engine::play_pack_once(&pack).map_err(|e| e.to_string())?
            {
                eprintln!("{warning}");
            }
            Ok(())
        }
        #[cfg(feature = "server")]
        Command::Status { pack } => {
            let world = spookery_engine::World::resume(&pack).map_err(|e| e.to_string())?;
            let text = world.host_summary().map_err(|e| e.to_string())?;
            print!("{text}");
            Ok(())
        }
        #[cfg(feature = "server")]
        Command::Migrate { pack } => {
            spookery_engine::migrate_pack(&pack).map_err(|e| e.to_string())
        }
        #[cfg(feature = "server")]
        Command::Verify { pack, from, quick } => {
            if quick && from.is_some() {
                return Err("verify --quick does not take --from".into());
            }
            let report = if quick {
                spookery_engine::verify_quick(&pack)
            } else {
                spookery_engine::verify_pack(&pack, from.as_deref())
            }
            .map_err(|e| e.to_string())?;
            println!(
                "verified {} records, state_rev {}",
                report.applied, report.state_rev
            );
            Ok(())
        }
        #[cfg(feature = "server")]
        Command::Check { pack, json, rules } => {
            let mut found = spookery_engine::check_pack(&pack);
            if rules && found.is_empty() {
                let canon = spookery_engine::canon::load_canon(&pack)
                    .map_err(|diags| spookery_engine::reason_of(diags).to_string())?;
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
                println!(
                    "{}",
                    serde_json::to_string(&found).expect("diagnostic json")
                );
            } else if found.is_empty() {
                println!("no problems");
            } else {
                for item in &found {
                    eprintln!("{}", item.report_line());
                }
            }
            if found.iter().any(spookery_engine::Diagnostic::is_error) {
                std::process::exit(1);
            }
            Ok(())
        }
        #[cfg(feature = "server")]
        Command::New { dir } => {
            let next = spookery::starter::write_starter(&dir).map_err(|e| e.to_string())?;
            print!("{next}");
            Ok(())
        }
    }
}

#[cfg(feature = "client")]
fn from_terminal(result: Result<(), spookery::terminal::Error>) -> Result<(), String> {
    result.map_err(|err| match err {
        spookery::terminal::Error::Reason(code) => code.to_string(),
        spookery::terminal::Error::Io(err) => err.to_string(),
    })
}

fn parse_loopback(addr: &str) -> Result<SocketAddr, String> {
    if addr == "127.0.0.1" {
        return Ok(SocketAddr::from(([127, 0, 0, 1], 9737)));
    }
    let port = addr
        .strip_prefix("127.0.0.1:")
        .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| "NOT_IN_SPEC".to_string())?
        .parse::<u16>()
        .map_err(|_| "NOT_IN_SPEC".to_string())?;
    Ok(SocketAddr::from(([127, 0, 0, 1], port)))
}

fn door_addr(mcp_http: Option<String>) -> Result<Option<SocketAddr>, String> {
    mcp_http.as_deref().map(parse_loopback).transpose()
}

#[cfg(all(test, feature = "server", feature = "client"))]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn the_default_address_is_loopback_9737() {
        let cli = Cli::try_parse_from(["spook", "tick", "pack"]).unwrap();
        assert_eq!(cli.addr, "127.0.0.1:9737");
        assert_eq!(
            parse_loopback("127.0.0.1").unwrap(),
            SocketAddr::from(([127, 0, 0, 1], 9737))
        );
        assert_eq!(parse_loopback(&cli.addr).unwrap().port(), 9737);
        assert_eq!(parse_loopback("127.0.0.1:0").unwrap().port(), 0);
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
                let cli = Cli::try_parse_from(["spook", command, "--addr", addr])
                    .or_else(|_| {
                        Cli::try_parse_from(["spook", command, "--addr", addr, "no-such-pack"])
                    })
                    .unwrap();
                assert_eq!(
                    dispatch(cli).unwrap_err(),
                    "NOT_IN_SPEC",
                    "{command} --addr {addr}"
                );
            }
        }
    }

    #[test]
    fn usage_is_not_an_engine_refusal() {
        assert!(Cli::try_parse_from(["spook", "rnu", "x"]).is_err());
        assert!(Cli::try_parse_from(["spook", "gm", "pack"]).is_err());
        let gm = Cli::try_parse_from(["spook", "gm"]).unwrap();
        assert!(matches!(gm.command, Command::Gm { raw: false, .. }));
        let gm_raw = Cli::try_parse_from(["spook", "gm", "--raw"]).unwrap();
        assert!(matches!(gm_raw.command, Command::Gm { raw: true, .. }));
        assert!(Cli::try_parse_from(["spook", "run", "--as", "player:rowan"]).is_err());
        assert!(Cli::try_parse_from(["spook"]).is_err());
        assert!(Cli::try_parse_from(["spook", "mcp"]).is_err());
        assert!(Cli::try_parse_from(["spook", "mcp", "--addr", "127.0.0.1:9737"]).is_err());
        let http = Cli::try_parse_from(["spook", "run", "--mcp-http", "0.0.0.0:9", "no-such-pack"])
            .unwrap();
        assert_eq!(dispatch(http).unwrap_err(), "NOT_IN_SPEC");
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
            let open = Cli::try_parse_from(["spook", command, "--mcp-http", "0.0.0.0:9"]).unwrap();
            assert_eq!(
                dispatch(open).unwrap_err(),
                "NOT_IN_SPEC",
                "{command} --mcp-http"
            );
            let door =
                Cli::try_parse_from(["spook", command, "--mcp-http", "127.0.0.1:9801"]).unwrap();
            let text = match door.command {
                Command::Gm { mcp_http, .. } | Command::AiGm { mcp_http, .. } => mcp_http,
                _ => panic!("{command}"),
            };
            assert_eq!(text.as_deref(), Some("127.0.0.1:9801"));
        }
        let play = Cli::try_parse_from(["spook", "play", "--as", "pc_rowan"]).unwrap();
        match play.command {
            Command::Play { as_player, raw } => {
                assert_eq!(as_player.as_deref(), Some("pc_rowan"));
                assert!(!raw);
            }
            _ => panic!("play"),
        }
        let raw = Cli::try_parse_from(["spook", "play", "--raw"]).unwrap();
        assert!(matches!(raw.command, Command::Play { raw: true, .. }));
        let host = Cli::try_parse_from(["spook", "ai-gm"]).unwrap();
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
