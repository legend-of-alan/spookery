use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn spook(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_spook"))
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("spawn spook: {err}"))
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap_or_else(|err| panic!("mkdir {}: {err}", to.display()));
    for entry in fs::read_dir(from).unwrap_or_else(|err| panic!("read {}: {err}", from.display())) {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            fs::copy(entry.path(), &dest)
                .unwrap_or_else(|err| panic!("copy {}: {err}", entry.path().display()));
        }
    }
}

fn lamp_oil(pack: &Path) -> i64 {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    shard
        .now()
        .places
        .get("docks_shop")
        .and_then(|place| place.stock.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("lamp_oil missing"))
}

fn stock_rate(pack: &Path) -> i64 {
    let shard = spookery_kernel::open_shard(pack).unwrap_or_else(|err| panic!("open_shard: {err}"));
    shard
        .canon()
        .profile("mira")
        .and_then(|profile| profile.stock_rate.get("docks_shop"))
        .and_then(|rates| rates.get("lamp_oil"))
        .copied()
        .unwrap_or_else(|| panic!("stock_rate for lamp_oil missing"))
}

#[test]
fn tick_03_shop_lives() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-tick03-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);
    let pack_arg = pack.to_str().unwrap();

    let rate = stock_rate(&pack);
    assert_eq!(rate, 4, "docks lamp_oil stock_rate");
    let mut previous = lamp_oil(&pack);
    assert!(previous >= 0, "start stock {previous}");

    for tick in 1..=13 {
        let output = spook(&["tick", pack_arg]);
        assert!(
            output.status.success(),
            "tick {tick}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let qty = lamp_oil(&pack);
        let delta = qty - previous;
        assert!(
            delta.abs() <= rate,
            "tick {tick}: lamp_oil {previous} -> {qty} exceeds stock_rate {rate}"
        );
        assert!(qty >= 0, "tick {tick}: lamp_oil {qty}");
        previous = qty;
    }

    let _ = fs::remove_dir_all(&pack);
}
