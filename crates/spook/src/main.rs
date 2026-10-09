use std::net::SocketAddr;
use std::path::Path;

use spookery_kernel::Reason;

fn main() {
    if let Err(exit) = dispatch() {
        match exit {
            spookery::Error::Reason(code) => eprintln!("{code}"),
            spookery::Error::Io(err) => eprintln!("{err}"),
        }
        std::process::exit(1);
    }
}

fn dispatch() -> Result<(), spookery::Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--version"] {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() == 2 && args[0] == "tick" {
        return spookery_tick::tick_pack(Path::new(&args[1])).map_err(spookery::Error::Reason);
    }
    if args.len() == 2 && args[0] == "migrate" {
        return spookery_kernel::commit::migrate_pack(Path::new(&args[1]))
            .map_err(spookery::Error::Reason);
    }
    if args.len() == 2 && args[0] == "status" {
        let shard =
            spookery_kernel::open_shard(Path::new(&args[1])).map_err(spookery::Error::Reason)?;
        let text = spookery_kernel::idle_digest(&shard, spookery_kernel::DigestViewer::Host)
            .map_err(spookery::Error::Reason)?;
        print!("{text}");
        return Ok(());
    }
    if args.first().is_some_and(|arg| arg == "run") {
        return dispatch_run(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "play") {
        return from_cli(spookery::cli::play(&args[1..]));
    }
    if args.first().is_some_and(|arg| arg == "ai-gm") {
        if args.len() != 2 {
            return Err(spookery::Error::Reason(Reason::NotInSpec));
        }
        if spookery::ai::sit(Path::new(&args[1])).is_err() {
            std::process::exit(1);
        }
        return Ok(());
    }
    Err(spookery::Error::Reason(Reason::NotInSpec))
}

fn from_cli(result: Result<(), spookery::cli::Error>) -> Result<(), spookery::Error> {
    result.map_err(|err| match err {
        spookery::cli::Error::Reason(code) => spookery::Error::Reason(code),
        spookery::cli::Error::Io(err) => spookery::Error::Io(err),
    })
}

fn dispatch_run(args: &[String]) -> Result<(), spookery::Error> {
    if args.is_empty() {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    }
    let pack = Path::new(&args[0]);
    let flags = parse_run_flags(&args[1..])?;
    let addr = match flags.bind {
        Some(addr) => parse_loopback(addr)?,
        None => SocketAddr::from(([127, 0, 0, 1], 9737)),
    };
    spookery::seat::listen(pack, addr)
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

struct RunFlags<'a> {
    bind: Option<&'a str>,
}

fn parse_run_flags(flags: &[String]) -> Result<RunFlags<'_>, spookery::Error> {
    if flags.iter().any(|flag| flag == "--as") {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    }
    let mut bind = None;
    let mut index = 0;
    while index < flags.len() {
        match flags[index].as_str() {
            "--bind" => {
                if bind.is_some() || index + 1 >= flags.len() {
                    return Err(spookery::Error::Reason(Reason::NotInSpec));
                }
                bind = Some(flags[index + 1].as_str());
                index += 2;
            }
            _ => return Err(spookery::Error::Reason(Reason::NotInSpec)),
        }
    }
    Ok(RunFlags { bind })
}
