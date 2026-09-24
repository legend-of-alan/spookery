use std::path::Path;

use spookery_kernel::Code;
use spookery_mcp as _;

fn main() {
    if let Err(code) = dispatch() {
        eprintln!("{code}");
        std::process::exit(1);
    }
}

fn dispatch() -> Result<(), Code> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--version"] {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.len() == 2 && args[0] == "tick" {
        return spookery_tick::tick_pack(Path::new(&args[1]));
    }
    if args.len() == 2 && args[0] == "status" {
        let shard = spookery_kernel::open_shard(Path::new(&args[1]))?;
        let text = spookery_kernel::idle_digest(&shard, spookery_kernel::DigestViewer::Host)?;
        print!("{text}");
        return Ok(());
    }
    if args.len() == 4 && args[0] == "run" && args[2] == "--mcp" {
        if let Some(profile) = spookery_mcp::parse_profile(&args[3]) {
            return spookery_mcp::serve(Path::new(&args[1]), profile);
        }
    }
    Err(Code::NotInSpec)
}
