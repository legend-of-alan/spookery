use spookery_kernel as _;
use spookery_mcp as _;
use spookery_tick as _;

fn main() {
    let mut args = std::env::args();
    let _argv0 = args.next();
    let args: Vec<String> = args.collect();
    if args.as_slice() == ["--version"] {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    eprintln!("NOT_IN_SPEC");
    std::process::exit(1);
}
