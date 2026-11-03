//! The Spookery table server: bind the game port's WebSocket door and the MCP endpoint,
//! then run the table.

mod frames;
mod idle;
mod limits;
mod mcp;
mod seat;
mod table;
mod tools;
mod ws;

use std::future::Future;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;

use spookery_engine::Reason;
use spookery_protocol as protocol;

use table::{Request, Table};

#[derive(Debug)]
pub enum Error {
    Reason(Reason),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Reason(code) => write!(f, "{code}"),
            Self::Io(err) => write!(f, "{err}"),
        }
    }
}

pub struct Config {
    pub addr: SocketAddr,
    pub mcp_addr: Option<SocketAddr>,
    pub hello_timeout: Duration,
    pub out: Box<dyn Write + Send>,
    pub err: Box<dyn Write + Send>,
}

impl Config {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            mcp_addr: None,
            hello_timeout: limits::HELLO_TIMEOUT,
            out: Box::new(io::sink()),
            err: Box::new(io::sink()),
        }
    }
}

/// Bind both ports, report them and serve on the caller's runtime until
/// `shutdown` resolves. `world` owns the pack lock until the table ends.
/// Returns the world's result.
pub async fn serve(
    world: spookery_engine::World,
    mut config: Config,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    let listener = bind(config.addr).await?;
    let mcp_listener = bind(mcp_bind_addr(config.addr, config.mcp_addr)?).await?;
    let game = listener.local_addr().map_err(Error::Io)?;
    let mcp_addr = mcp_listener.local_addr().map_err(Error::Io)?;
    print_addrs(&mut *config.out, game, mcp_addr)?;
    let idle_rounds = world.budget().idle_rounds;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut tasks = tokio::task::JoinSet::new();
    let next_conn = Arc::new(AtomicU64::new(1));
    tasks.spawn(ws::serve(
        listener,
        tx.clone(),
        Arc::clone(&next_conn),
        config.hello_timeout,
    ));
    tasks.spawn(mcp::serve(mcp_listener, tx.clone(), next_conn));
    let (restart_tx, restart_rx) = tokio::sync::watch::channel(0u64);
    if idle_rounds {
        tasks.spawn(idle::run(
            tx.clone(),
            restart_rx,
            world.budget().round_every_real_seconds,
        ));
    }
    let mut table = tokio::task::spawn_blocking(move || {
        Table::new(world, restart_tx, config.out, config.err).run(rx)
    });
    // A table that ends on its own (a store fault) ends `serve` too.
    let done = tokio::select! {
        done = &mut table => done,
        () = shutdown => {
            let _ = tx.send(Request::Stop);
            table.await
        }
    };
    done.unwrap_or(Err(Error::Reason(Reason::NotInSpec)))
}

async fn bind(addr: SocketAddr) -> Result<tokio::net::TcpListener, Error> {
    tokio::net::TcpListener::bind(addr).await.map_err(Error::Io)
}

/// The MCP address to bind. A fixed game port keeps the next port. Port 0, on
/// the game address or on `--mcp-addr`, asks the OS for a port of its own.
fn mcp_bind_addr(game: SocketAddr, mcp_addr: Option<SocketAddr>) -> Result<SocketAddr, Error> {
    if let Some(mcp_addr) = mcp_addr {
        return Ok(mcp_addr);
    }
    if game.port() == 0 {
        return Ok(SocketAddr::new(game.ip(), 0));
    }
    protocol::default_mcp_addr(game).ok_or(Error::Reason(Reason::NotInSpec))
}

fn print_addrs(out: &mut dyn Write, game: SocketAddr, mcp_addr: SocketAddr) -> Result<(), Error> {
    let line = serde_json::to_string(&protocol::Addrs {
        game: game.to_string(),
        mcp: mcp_addr.to_string(),
    })
    .map_err(|err| Error::Io(err.into()))?;
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(Error::Io)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::mcp_bind_addr;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    #[test]
    fn a_fixed_game_port_uses_the_next_port() {
        assert_eq!(mcp_bind_addr(addr(9737), None).ok(), Some(addr(9738)));
    }

    #[test]
    fn port_zero_asks_for_an_mcp_port_of_its_own() {
        assert_eq!(mcp_bind_addr(addr(0), None).ok(), Some(addr(0)));
    }

    #[test]
    fn an_explicit_mcp_addr_wins() {
        assert_eq!(mcp_bind_addr(addr(0), Some(addr(9))).ok(), Some(addr(9)));
        assert_eq!(mcp_bind_addr(addr(9737), Some(addr(0))).ok(), Some(addr(0)));
    }

    #[test]
    fn the_last_port_has_no_next_port() {
        assert!(mcp_bind_addr(addr(65535), None).is_err());
    }
}
