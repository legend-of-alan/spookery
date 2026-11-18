//! The `spook` command for one loopback table.

mod new;

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

/// `--version`: the release workflow sets `SPOOK_RELEASE` to the tag and commit, as
/// `alpha-2026.11.14 (97960932)`. A local build shows the Cargo version.
const VERSION: &str = match option_env!("SPOOK_RELEASE") {
    Some(release) => release,
    None => env!("CARGO_PKG_VERSION"),
};

/// One loopback table.
#[derive(Parser)]
#[command(name = "spook", version = VERSION)]
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
    ///
    /// A second Ctrl+C exits at once when shutdown hangs.
    #[cfg(feature = "server")]
    Run {
        /// Pack directory.
        pack: PathBuf,
        /// MCP endpoint address. Defaults to the game port plus one. Port 0 lets the OS choose.
        #[arg(long = "mcp-addr", value_name = "127.0.0.1:<port>")]
        mcp_addr: Option<String>,
    },
    /// Sit the player seat.
    #[cfg(feature = "client")]
    Play {
        /// Print each protocol frame as one JSON line.
        #[arg(long)]
        raw: bool,
    },
    /// Sit the GM seat.
    #[cfg(feature = "client")]
    Gm {
        /// Print each protocol frame as one JSON line.
        #[arg(long)]
        raw: bool,
    },
    /// Sit the GM seat on the game port and call the model.
    #[cfg(feature = "client")]
    #[command(name = "ai-gm")]
    AiGm,
    /// Play one round on the pack.
    #[cfg(feature = "server")]
    Round {
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
    New {
        /// Directory to create. An existing non-empty directory is refused.
        #[arg(value_name = "DIR")]
        dir: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(exit) = dispatch(cli).await {
        eprintln!("{exit}");
        std::process::exit(1);
    }
}

async fn dispatch(cli: Cli) -> Result<(), String> {
    let addr = parse_loopback(&cli.addr)?;
    #[cfg_attr(not(feature = "server"), allow(unused_variables))]
    let log_path = start_logging(&cli.command)?;
    match cli.command {
        #[cfg(feature = "server")]
        Command::Run { pack, mcp_addr } => {
            let mcp_addr = parse_mcp_addr(mcp_addr)?;
            let lock = spookery_engine::lock_pack(&pack).map_err(|e| e.to_string())?;
            let session =
                spookery_engine::Session::resume(&pack, lock).map_err(|e| e.to_string())?;
            if let Some(path) = &log_path {
                eprintln!("logging to {}", path.display());
            }
            let mut config = spookery_server::Config::new(addr);
            config.mcp_addr = mcp_addr;
            config.out = Box::new(std::io::stdout());
            let ctrl_c = tokio::spawn(tokio::signal::ctrl_c());
            let shutdown = async {
                if let Err(err) = ctrl_c.await.map_err(std::io::Error::from).and_then(|r| r) {
                    eprintln!("{err}");
                }
                tokio::spawn(async {
                    let _ = tokio::signal::ctrl_c().await;
                    std::process::exit(130);
                });
            };
            spookery_server::serve(session, config, shutdown)
                .await
                .map_err(|e| e.to_string())
        }
        #[cfg(feature = "client")]
        Command::Play { raw } => spookery::play::run(addr, raw)
            .await
            .map_err(|err| err.to_string()),
        #[cfg(feature = "client")]
        Command::Gm { raw } => spookery::gm::run(addr, raw)
            .await
            .map_err(|err| err.to_string()),
        #[cfg(feature = "client")]
        Command::AiGm => {
            let model = spookery::http_model::Settings::from_env()
                .and_then(spookery::http_model::HttpModel::new)
                .map_err(|err| err.to_string())?;
            spookery::ai_gm::run(addr, model, std::io::stderr())
                .await
                .map_err(|err| err.to_string())
        }
        #[cfg(feature = "server")]
        Command::Round { pack } => {
            spookery_engine::play_pack_once(&pack).map_err(|e| e.to_string())
        }
        #[cfg(feature = "server")]
        Command::Status { pack } => {
            let text = spookery_engine::Session::peek_digest(&pack).map_err(|e| e.to_string())?;
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
            for rev in report.trusted_edits {
                println!("operator edit trusted at state_rev {rev}");
            }
            Ok(())
        }
        #[cfg(feature = "server")]
        Command::Check { pack, json, rules } => {
            let mut found = spookery_engine::check_pack(&pack);
            if rules && found.is_empty() {
                let canon = spookery_engine::load_canon(&pack)
                    .map_err(|diags| spookery_engine::first_reason(&diags).to_string())?;
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
                    eprintln!("{item}");
                }
            }
            if found.iter().any(spookery_engine::Diagnostic::is_error) {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::New { dir } => {
            let next = new::write(&dir).map_err(|e| e.to_string())?;
            print!("{next}");
            Ok(())
        }
    }
}

/// Installs the one log subscriber. `spook run` and `spook ai-gm` log to a new file under
/// `logs/`, whose path this returns. Every other command logs to stderr. A `RUST_LOG` whose
/// directives are all invalid logs warnings.
fn start_logging(command: &Command) -> Result<Option<PathBuf>, String> {
    let (command_id, default_directives) = match command {
        #[cfg(feature = "server")]
        Command::Run { .. } => (Some("run"), "warn,spook=info"),
        #[cfg(feature = "client")]
        Command::AiGm => (Some("ai-gm"), "warn,spook=info"),
        _ => (None, "warn"),
    };
    let directives = std::env::var("RUST_LOG")
        .ok()
        .filter(|text| !text.is_empty());
    let filter = tracing_subscriber::EnvFilter::builder()
        .with_default_directive(tracing_subscriber::filter::LevelFilter::WARN.into())
        .parse_lossy(directives.as_deref().unwrap_or(default_directives));
    let subscriber = tracing_subscriber::fmt().with_env_filter(filter);
    let Some(command_id) = command_id else {
        subscriber.with_writer(std::io::stderr).init();
        return Ok(None);
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = format!("{command_id}-{stamp}-{}.log", std::process::id());
    let path = std::path::absolute(Path::new("logs").join(name)).map_err(|err| err.to_string())?;
    fs::create_dir_all("logs").map_err(|err| format!("logs: {err}"))?;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("{}: {err}", path.display()))?;
    subscriber.with_writer(file).init();
    Ok(Some(path))
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

#[cfg(feature = "server")]
fn parse_mcp_addr(mcp_addr: Option<String>) -> Result<Option<SocketAddr>, String> {
    mcp_addr.as_deref().map(parse_loopback).transpose()
}

#[cfg(all(test, feature = "server", feature = "client"))]
mod tests {
    use std::path::Path;

    use super::*;

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
    fn usage_is_not_an_engine_refusal() {
        assert!(Cli::try_parse_from(["spook", "rnu", "x"]).is_err());
        assert!(Cli::try_parse_from(["spook", "gm", "pack"]).is_err());
        let gm = Cli::try_parse_from(["spook", "gm"]).unwrap();
        assert!(matches!(gm.command, Command::Gm { raw: false }));
        let gm_raw = Cli::try_parse_from(["spook", "gm", "--raw"]).unwrap();
        assert!(matches!(gm_raw.command, Command::Gm { raw: true }));
        assert!(Cli::try_parse_from(["spook"]).is_err());
        let open = parse_mcp_addr(Some("0.0.0.0:9".into()));
        assert_eq!(open.unwrap_err(), "NOT_IN_SPEC");
        let http_loopback =
            Cli::try_parse_from(["spook", "run", "--mcp-addr", "127.0.0.1:9801", "pack"]).unwrap();
        assert!(matches!(
            http_loopback.command,
            Command::Run {
                mcp_addr: Some(text),
                ..
            } if text == "127.0.0.1:9801"
        ));
        assert!(Cli::try_parse_from(["spook", "gm", "--mcp-addr", "127.0.0.1:9801"]).is_err());
        let play = Cli::try_parse_from(["spook", "play"]).unwrap();
        assert!(matches!(play.command, Command::Play { raw: false }));
        let raw = Cli::try_parse_from(["spook", "play", "--raw"]).unwrap();
        assert!(matches!(raw.command, Command::Play { raw: true }));
        let host = Cli::try_parse_from(["spook", "ai-gm"]).unwrap();
        assert!(matches!(host.command, Command::AiGm));
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

    #[test]
    fn every_spook_line_in_the_readme_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../README.md");
        let readme = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
        let mut found = false;
        let mut fenced = false;
        for line in readme.lines() {
            if line.starts_with("```") {
                fenced = !fenced;
                continue;
            }
            if !fenced {
                continue;
            }
            let Some(args) = split_spook_line(line) else {
                continue;
            };
            found = true;
            Cli::try_parse_from(&args).unwrap_or_else(|err| panic!("{}: {err}", args.join(" ")));
        }
        assert!(found, "README.md has no spook command");
    }

    fn split_spook_line(line: &str) -> Option<Vec<&str>> {
        let args: Vec<&str> = line.split_whitespace().collect();
        ["spook", "./spook"].contains(args.first()?).then_some(args)
    }
}
