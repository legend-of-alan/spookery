mod play;

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
        return play::play(&args[1..]);
    }
    if args.first().is_some_and(|arg| arg == "gm") {
        return play::gm(&args[1..]);
    }
    Err(spookery::Error::Reason(Reason::NotInSpec))
}

fn dispatch_run(args: &[String]) -> Result<(), spookery::Error> {
    if args.is_empty() {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    }
    let pack = Path::new(&args[0]);
    let flags = parse_run_flags(&args[1..])?;
    if let Some(profile) = flags.mcp {
        let Some(profile) = spookery_mcp::parse_profile(profile) else {
            return Err(spookery::Error::Reason(Reason::NotInSpec));
        };
        return spookery_mcp::serve(pack, profile).map_err(spookery::Error::Reason);
    }
    if let Some(addr) = flags.bind {
        if addr != "127.0.0.1" {
            return Err(spookery::Error::Reason(Reason::NotInSpec));
        }
    }
    spookery::seat::listen(pack)
}

struct RunFlags<'a> {
    mcp: Option<&'a str>,
    bind: Option<&'a str>,
}

fn parse_run_flags(flags: &[String]) -> Result<RunFlags<'_>, spookery::Error> {
    if flags.iter().any(|flag| flag == "--as") {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    }
    let mut mcp = None;
    let mut bind = None;
    let mut index = 0;
    while index < flags.len() {
        match flags[index].as_str() {
            "--mcp" => {
                if mcp.is_some() || index + 1 >= flags.len() {
                    return Err(spookery::Error::Reason(Reason::NotInSpec));
                }
                mcp = Some(flags[index + 1].as_str());
                index += 2;
            }
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
    if mcp.is_some() && bind.is_some() {
        return Err(spookery::Error::Reason(Reason::NotInSpec));
    }
    Ok(RunFlags { mcp, bind })
}
