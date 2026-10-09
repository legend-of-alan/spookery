use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::Path;

use spookery_kernel::Reason;
use spookery_protocol::PlayerMessage;

pub enum Error {
    Reason(Reason),
    Io(io::Error),
}

pub fn play(args: &[String]) -> Result<(), Error> {
    if args.is_empty() {
        return Err(Error::Reason(Reason::NotInSpec));
    }
    let pack = Path::new(&args[0]);
    let actor = if args.len() == 1 {
        start_pc(pack)?
    } else if args.len() == 3 && args[1] == "--as" {
        parse_player_as(&args[2])?
    } else {
        return Err(Error::Reason(Reason::NotInSpec));
    };
    let mut stream = TcpStream::connect("127.0.0.1:9737").map_err(Error::Io)?;
    stream.set_nodelay(true).map_err(Error::Io)?;
    stream
        .write_all(spookery_protocol::write_player_hello(&actor).as_bytes())
        .map_err(Error::Io)?;
    stream.flush().map_err(Error::Io)?;
    let mut reader = BufReader::new(stream);
    loop {
        let Some(line) = read_line(&mut reader)? else {
            eprintln!("{}", Reason::NotInSpec);
            std::process::exit(1);
        };
        if let Some(code) = refused_code(&line) {
            eprintln!("{code}");
            std::process::exit(1);
        }
        println!("{line}");
        let _ = io::stdout().flush();
        if is_perceive(&line) {
            break;
        }
    }
    let mut write_stream = reader.get_mut().try_clone().map_err(Error::Io)?;
    std::thread::spawn(move || loop {
        match read_line(&mut reader) {
            Ok(Some(line)) => {
                if let Some(code) = refused_code(&line) {
                    eprintln!("{code}");
                    let _ = io::stderr().flush();
                    continue;
                }
                println!("{line}");
                let _ = io::stdout().flush();
            }
            Ok(None) => std::process::exit(0),
            Err(Error::Io(err)) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
            Err(Error::Reason(code)) => {
                eprintln!("{code}");
                std::process::exit(1);
            }
        }
    });
    for input in io::stdin().lines() {
        let input = input.map_err(Error::Io)?;
        writeln!(write_stream, "{input}").map_err(Error::Io)?;
        write_stream.flush().map_err(Error::Io)?;
    }
    Ok(())
}

fn parse_player_as(token: &str) -> Result<String, Error> {
    let Some(id) = token.strip_prefix("player:") else {
        return Err(Error::Reason(Reason::NotInSpec));
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(Error::Reason(Reason::Schema));
    }
    Ok(id.to_string())
}

fn start_pc(pack: &Path) -> Result<String, Error> {
    let id = match spookery_kernel::canon::load_canon(pack) {
        Ok(canon) => canon.start_pc,
        Err(Reason::NotInSpec) => pack_pc_line(pack)?,
        Err(code) => return Err(Error::Reason(code)),
    };
    if id.is_empty() || id.chars().any(char::is_whitespace) {
        return Err(Error::Reason(Reason::Schema));
    }
    Ok(id)
}

fn pack_pc_line(pack: &Path) -> Result<String, Error> {
    let text =
        fs::read_to_string(pack.join("pack.yaml")).map_err(|_| Error::Reason(Reason::NotInSpec))?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("pc:") else {
            continue;
        };
        let mut id = rest.trim();
        if id.len() >= 2 && id.starts_with('"') && id.ends_with('"') {
            id = &id[1..id.len() - 1];
        }
        return Ok(id.to_string());
    }
    Err(Error::Reason(Reason::Schema))
}

fn read_line(reader: &mut BufReader<TcpStream>) -> Result<Option<String>, Error> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(Error::Io)?;
    if read == 0 {
        return Ok(None);
    }
    if line.ends_with('\n') {
        line.pop();
    }
    if line.ends_with('\r') {
        line.pop();
    }
    Ok(Some(line))
}

fn refused_code(line: &str) -> Option<String> {
    match spookery_protocol::read_player(line) {
        Ok(PlayerMessage::Refused { code }) => Some(code),
        _ => None,
    }
}

fn is_perceive(line: &str) -> bool {
    matches!(
        spookery_protocol::read_player(line),
        Ok(PlayerMessage::Perceive { .. })
    )
}
