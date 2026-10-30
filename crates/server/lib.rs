//! The Spookery table server: bind the game port and the agent door, then run the table.

mod door;
mod frames;
mod idle;
mod limits;
mod player;
mod seat;
mod table;
mod tools;

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
    pub door: Option<SocketAddr>,
    pub hello_timeout: Duration,
    pub out: Box<dyn Write + Send>,
    pub err: Box<dyn Write + Send>,
}

impl Config {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            door: None,
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
    let http_listener = bind(door_addr(config.addr, config.door)?).await?;
    let game = listener.local_addr().map_err(Error::Io)?;
    let door = http_listener.local_addr().map_err(Error::Io)?;
    report_bound(&mut *config.out, game, door)?;
    // Keepalive stays the pack interval. The idle switch does not change it.
    let keepalive_secs = world.budget().round_every_real_seconds;
    let arm_idle_timer = world.budget().idle_rounds;
    let (tx, rx) = std::sync::mpsc::channel();
    let mut tasks = tokio::task::JoinSet::new();
    let next_conn = Arc::new(AtomicU64::new(1));
    tasks.spawn(player::serve(
        listener,
        tx.clone(),
        Arc::clone(&next_conn),
        keepalive_secs,
        config.hello_timeout,
    ));
    tasks.spawn(door::serve(http_listener, tx.clone(), next_conn));
    let (restart_tx, restart_rx) = tokio::sync::watch::channel(0u64);
    if arm_idle_timer {
        tasks.spawn(idle::run(tx.clone(), restart_rx, keepalive_secs));
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

/// The agent door to bind. A fixed game port keeps the next port. Port 0, on
/// the game address or on `--door`, asks the OS for a port of its own.
fn door_addr(game: SocketAddr, mcp_addr: Option<SocketAddr>) -> Result<SocketAddr, Error> {
    if let Some(door) = mcp_addr {
        return Ok(door);
    }
    if game.port() == 0 {
        return Ok(SocketAddr::new(game.ip(), 0));
    }
    protocol::default_door(game).ok_or(Error::Reason(Reason::NotInSpec))
}

fn report_bound(out: &mut dyn Write, game: SocketAddr, door: SocketAddr) -> Result<(), Error> {
    let line = serde_json::to_string(&protocol::Listening {
        game: game.to_string(),
        door: door.to_string(),
    })
    .map_err(|err| Error::Io(err.into()))?;
    writeln!(out, "{line}")
        .and_then(|()| out.flush())
        .map_err(Error::Io)
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use super::door_addr;

    fn addr(port: u16) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
    }

    #[test]
    fn a_fixed_game_port_uses_the_next_port() {
        assert_eq!(door_addr(addr(9737), None).ok(), Some(addr(9738)));
    }

    #[test]
    fn port_zero_asks_for_a_door_of_its_own() {
        assert_eq!(door_addr(addr(0), None).ok(), Some(addr(0)));
    }

    #[test]
    fn an_explicit_door_wins() {
        assert_eq!(door_addr(addr(0), Some(addr(9))).ok(), Some(addr(9)));
        assert_eq!(door_addr(addr(9737), Some(addr(0))).ok(), Some(addr(0)));
    }

    #[test]
    fn the_last_port_has_no_next_port() {
        assert!(door_addr(addr(65535), None).is_err());
    }
}
