use spookery_engine::{Reason, World};
use spookery_server::{Config, Error};
use std::{fs, io::*, net::*, path::Path};

fn copy_dir(from: &Path, to: &Path) {
    let _ = fs::remove_dir_all(to);
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        match (e.path(), to.join(e.file_name())) {
            (p, dst) if p.is_dir() => copy_dir(&p, &dst),
            (p, dst) => _ = fs::copy(p, dst).unwrap(),
        }
    }
}

fn step(c: &mut BufReader<TcpStream>, cmd: &str) {
    c.get_mut().write_all(cmd.as_bytes()).unwrap();
    let mut line = String::new();
    while !line.contains("\"perceive\"") {
        line.clear();
        assert!(c.read_line(&mut line).unwrap() > 0);
    }
}

#[test]
fn two_worlds_serve_on_port_zero_and_stop_independently() {
    let docks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
    let first_dir = std::env::temp_dir().join(format!("spook-2w-1-{}", std::process::id()));
    let second_dir = std::env::temp_dir().join(format!("spook-2w-2-{}", std::process::id()));
    copy_dir(&docks, &first_dir);
    copy_dir(&docks, &second_dir);
    let loopback_0: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let world1 = World::resume(&first_dir).unwrap();
    let first = spookery_server::serve(world1, Config::new(loopback_0)).unwrap();
    let err = spookery_server::serve(World::resume(&first_dir).unwrap(), Config::new(loopback_0));
    assert!(matches!(err, Err(Error::Reason(Reason::CommitConflict))));
    let world2 = World::resume(&second_dir).unwrap();
    let second = spookery_server::serve(world2, Config::new(loopback_0)).unwrap();
    let mut c1 = BufReader::new(TcpStream::connect(first.game_addr()).unwrap());
    let mut c2 = BufReader::new(TcpStream::connect(second.game_addr()).unwrap());
    step(&mut c1, "player:pc_rowan\n");
    step(&mut c1, "wait\n");
    step(&mut c2, "player:pc_rowan\n");
    step(&mut c2, "wait\n");
    assert_eq!(World::resume(&first_dir).unwrap().now().tick_index, 1);
    assert_eq!(World::resume(&second_dir).unwrap().now().tick_index, 1);
    first.stop();
    first.join().unwrap();
    step(&mut c2, "wait\n");
    assert_eq!(World::resume(&second_dir).unwrap().now().tick_index, 2);
    second.stop();
    second.join().unwrap();
    fs::remove_dir_all(&first_dir).unwrap();
    fs::remove_dir_all(&second_dir).unwrap();
}
