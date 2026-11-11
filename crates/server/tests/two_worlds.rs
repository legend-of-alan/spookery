use spookery_engine::testing::copy_docks;
use spookery_engine::{lock_pack, Reason, World};
use spookery_server::{Config, Error};
use std::{fs, io::*, net::*, path::Path};
use tokio::sync::oneshot;
use tungstenite::{Message, WebSocket};

type Player = WebSocket<TcpStream>;

/// Sends one command and reads until a view arrives.
fn send_until_perceive(player: &mut Player, command: &str, args: serde_json::Value) {
    let message = serde_json::json!({ "id": command, "body": { "kind": command, "args": args } });
    player.send(Message::text(message.to_string())).unwrap();
    loop {
        let Message::Text(text) = player.read().unwrap() else {
            continue;
        };
        if text.contains("\"kind\":\"view\"") {
            return;
        }
    }
}

/// Serve `dir` on port 0 until the sender fires, and open a player seat on the game port
/// that the `listening` line reports.
fn host(
    dir: &Path,
) -> (
    Player,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<std::result::Result<(), Error>>,
) {
    let (reader, writer) = pipe().unwrap();
    let mut config = Config::new("127.0.0.1:0".parse().unwrap());
    config.out = Box::new(writer);
    let (stop, stopped) = oneshot::channel::<()>();
    let world = World::resume(dir, lock_pack(dir).unwrap()).unwrap();
    let served = tokio::spawn(spookery_server::serve(world, config, async {
        stopped.await.unwrap()
    }));
    let mut line = String::new();
    BufReader::new(reader).read_line(&mut line).unwrap();
    let addrs: serde_json::Value = serde_json::from_str(&line).unwrap();
    let game = addrs["game"].as_str().unwrap();
    let stream = TcpStream::connect(game).unwrap();
    let (player, _) = tungstenite::client(format!("ws://{game}/player"), stream).unwrap();
    (player, stop, served)
}

#[tokio::test(flavor = "multi_thread")]
async fn two_worlds_serve_on_port_zero_and_stop_independently() {
    let first_dir = copy_docks("2w-1");
    let second_dir = copy_docks("2w-2");
    let (mut c1, stop1, first) = host(&first_dir);
    assert_eq!(lock_pack(&first_dir).err(), Some(Reason::CommitConflict));
    let (mut c2, stop2, second) = host(&second_dir);
    let rowan = serde_json::json!({ "actor": "rowan" });
    let none = serde_json::json!({});
    send_until_perceive(&mut c1, "claim", rowan.clone());
    send_until_perceive(&mut c1, "wait", none.clone());
    send_until_perceive(&mut c2, "claim", rowan);
    send_until_perceive(&mut c2, "wait", none.clone());
    assert_eq!(World::peek(&first_dir).unwrap().state().round_index, 1);
    assert_eq!(World::peek(&second_dir).unwrap().state().round_index, 1);
    stop1.send(()).unwrap();
    first.await.unwrap().unwrap();
    send_until_perceive(&mut c2, "wait", none);
    assert_eq!(World::peek(&second_dir).unwrap().state().round_index, 2);
    stop2.send(()).unwrap();
    second.await.unwrap().unwrap();
    fs::remove_dir_all(&first_dir).unwrap();
    fs::remove_dir_all(&second_dir).unwrap();
}
