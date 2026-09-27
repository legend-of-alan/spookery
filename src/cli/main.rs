mod play;

use std::path::Path;

use spookery_kernel::Code;

enum Exit {
    Code(Code),
    Os(std::io::Error),
}

fn main() {
    if let Err(exit) = dispatch() {
        match exit {
            Exit::Code(code) => eprintln!("{code}"),
            Exit::Os(err) => eprintln!("{err}"),
        }
        std::process::exit(1);
    }
}

fn dispatch() -> Result<(), Exit> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--version"] {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() == 2 && args[0] == "tick" {
        return spookery_tick::tick_pack(Path::new(&args[1])).map_err(Exit::Code);
    }
    if args.len() == 2 && args[0] == "migrate" {
        return spookery_kernel::store::migrate_pack(Path::new(&args[1])).map_err(Exit::Code);
    }
    if args.len() == 2 && args[0] == "status" {
        let shard = spookery_kernel::open_shard(Path::new(&args[1])).map_err(Exit::Code)?;
        let text = spookery_kernel::idle_digest(&shard, spookery_kernel::DigestViewer::Host).map_err(Exit::Code)?;
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
    Err(Exit::Code(Code::NotInSpec))
}

fn dispatch_run(args: &[String]) -> Result<(), Exit> {
    if args.is_empty() {
        return Err(Exit::Code(Code::NotInSpec));
    }
    let pack = Path::new(&args[0]);
    let flags = parse_run_flags(&args[1..])?;
    if let Some(profile) = flags.mcp {
        let Some(profile) = spookery_mcp::parse_profile(profile) else {
            return Err(Exit::Code(Code::NotInSpec));
        };
        return spookery_mcp::serve(pack, profile).map_err(Exit::Code);
    }
    if let Some(addr) = flags.bind {
        if addr != "127.0.0.1" {
            return Err(Exit::Code(Code::NotInSpec));
        }
    }
    spookery::seat::listen(pack).map_err(|fail| match fail {
        spookery::seat::Fail::Code(code) => Exit::Code(code),
        spookery::seat::Fail::Io(err) => Exit::Os(err),
    })
}

struct RunFlags<'a> {
    mcp: Option<&'a str>,
    bind: Option<&'a str>,
}

fn parse_run_flags(flags: &[String]) -> Result<RunFlags<'_>, Exit> {
    if flags.iter().any(|flag| flag == "--as") {
        return Err(Exit::Code(Code::NotInSpec));
    }
    let mut mcp = None;
    let mut bind = None;
    let mut index = 0;
    while index < flags.len() {
        match flags[index].as_str() {
            "--mcp" => {
                if mcp.is_some() || index + 1 >= flags.len() {
                    return Err(Exit::Code(Code::NotInSpec));
                }
                mcp = Some(flags[index + 1].as_str());
                index += 2;
            }
            "--bind" => {
                if bind.is_some() || index + 1 >= flags.len() {
                    return Err(Exit::Code(Code::NotInSpec));
                }
                bind = Some(flags[index + 1].as_str());
                index += 2;
            }
            _ => return Err(Exit::Code(Code::NotInSpec)),
        }
    }
    if mcp.is_some() && bind.is_some() {
        return Err(Exit::Code(Code::NotInSpec));
    }
    Ok(RunFlags { mcp, bind })
}
