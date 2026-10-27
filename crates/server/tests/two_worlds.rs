use spookery_engine::testing::temp_pack;
use spookery_engine::{Reason, World};
use spookery_server::{Config, Error};
use std::{fs, io::*, net::*};

fn step(c: &mut BufReader<TcpStream>, cmd: &str) {
    c.get_mut().write_all(cmd.as_bytes()).unwrap();
    let mut line = String::new();
    while !line.contains("\"perceive\"") {
        line.clear();
        assert!(c.read_line(&mut line).unwrap() > 0);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn two_worlds_serve_on_port_zero_and_stop_independently() {
    let first_dir = temp_pack("2w-1");
    let second_dir = temp_pack("2w-2");
    let loopback_0: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let world1 = World::resume(&first_dir).unwrap();
    let first = spookery_server::serve(world1, Config::new(loopback_0)).unwrap();
    let err = spookery_server::serve(World::resume(&first_dir).unwrap(), Config::new(loopback_0));
    assert!(matches!(err, Err(Error::Reason(Reason::CommitConflict))));
    let world2 = World::resume(&second_dir).unwrap();
    let second = spookery_server::serve(world2, Config::new(loopback_0)).unwrap();
    let mut c1 = BufReader::new(TcpStream::connect(first.game_addr()).unwrap());
    let mut c2 = BufReader::new(TcpStream::connect(second.game_addr()).unwrap());
    let (stop1, stopped1) = tokio::sync::oneshot::channel::<()>();
    let (stop2, stopped2) = tokio::sync::oneshot::channel::<()>();
    let first = tokio::spawn(first.run(async { stopped1.await.unwrap() }));
    let second = tokio::spawn(second.run(async { stopped2.await.unwrap() }));
    step(&mut c1, "player:pc_rowan\n");
    step(&mut c1, "wait\n");
    step(&mut c2, "player:pc_rowan\n");
    step(&mut c2, "wait\n");
    assert_eq!(World::resume(&first_dir).unwrap().now().tick_index, 1);
    assert_eq!(World::resume(&second_dir).unwrap().now().tick_index, 1);
    stop1.send(()).unwrap();
    first.await.unwrap().unwrap();
    step(&mut c2, "wait\n");
    assert_eq!(World::resume(&second_dir).unwrap().now().tick_index, 2);
    stop2.send(()).unwrap();
    second.await.unwrap().unwrap();
    fs::remove_dir_all(&first_dir).unwrap();
    fs::remove_dir_all(&second_dir).unwrap();
}
