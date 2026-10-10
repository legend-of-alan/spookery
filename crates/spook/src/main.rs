use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use spookery_kernel::Reason;

/// One loopback table.
#[derive(Parser)]
#[command(name = "spook", version)]
struct Cli {
    /// Loopback address. `127.0.0.1` means port 9737.
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
    /// Sit the GM seat and call the model.
    #[command(name = "ai-gm")]
    AiGm {
        /// Pack directory.
        pack: PathBuf,
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
    /// Report pack problems and write nothing.
    Check {
        /// Pack directory.
        pack: PathBuf,
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
        Command::Run { pack } => spookery::seat::listen(&pack, addr),
        Command::Play {
            pack,
            as_player,
            raw,
        } => from_cli(spookery::cli::play(
            &pack,
            as_player.as_deref(),
            &addr.to_string(),
            raw,
        )),
        Command::AiGm { pack } => {
            if spookery::ai::sit(&pack, &addr.to_string()).is_err() {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Tick { pack } => spookery_tick::tick_pack(&pack).map_err(spookery::Error::Reason),
        Command::Status { pack } => {
            let shard = spookery_kernel::open_shard(&pack).map_err(spookery::Error::Reason)?;
            let text = spookery_kernel::idle_digest(&shard, spookery_kernel::DigestViewer::Host)
                .map_err(spookery::Error::Reason)?;
            print!("{text}");
            Ok(())
        }
        Command::Migrate { pack } => {
            spookery_kernel::commit::migrate_pack(&pack).map_err(spookery::Error::Reason)
        }
        Command::Check { pack } => {
            let found = spookery_kernel::check_pack(&pack);
            if found.is_empty() {
                println!("no problems");
                Ok(())
            } else {
                for item in &found {
                    eprintln!("{}", item.report_line());
                }
                std::process::exit(1);
            }
        }
    }
}

fn from_cli(result: Result<(), spookery::cli::Error>) -> Result<(), spookery::Error> {
    result.map_err(|err| match err {
        spookery::cli::Error::Reason(code) => spookery::Error::Reason(code),
        spookery::cli::Error::Io(err) => spookery::Error::Io(err),
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
        for command in ["run", "play", "ai-gm", "tick", "status", "migrate", "check"] {
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
        assert!(Cli::try_parse_from(["spook", "run", "--as", "player:rowan"]).is_err());
        assert!(Cli::try_parse_from(["spook"]).is_err());
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
    }
}
