use std::fs;
use std::path::{Path, PathBuf};

const LEDGER: &str = "The lamp-oil ledger is short.";

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

fn temp_pack(label: &str) -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let pack = std::env::temp_dir().join(format!(
        "spook-lint-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&pack);
    copy_dir(&fixture, &pack);
    pack
}

fn append(pack: &Path, rel: &str, text: &str) {
    let path = pack.join(rel);
    let mut body = fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {rel}: {err}"));
    body.push_str(text);
    if !text.ends_with('\n') {
        body.push('\n');
    }
    fs::write(&path, body).unwrap_or_else(|err| panic!("write {rel}: {err}"));
}

#[test]
fn v0_c12_hidden_string_lint() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture = manifest.join("packs/oneshot-docks");
    let clean = spookery_kernel::lint::lint(&fixture).unwrap_or_else(|err| panic!("lint: {err}"));
    assert!(clean.is_empty(), "docks bodies hide the ledger sentence: {clean:?}");

    let owner = temp_pack("owner");
    append(&owner, "cast/mira.md", LEDGER);
    let hits = spookery_kernel::lint::lint(&owner).unwrap_or_else(|err| panic!("lint owner: {err}"));
    assert!(
        hits.iter().any(|hit| hit.path == "cast/mira.md" && hit.text == LEDGER),
        "owner body was exempt: {hits:?}"
    );
    let _ = fs::remove_dir_all(&owner);

    let place = temp_pack("place");
    append(&place, "world/places/docks_shop.md", LEDGER);
    let hits = spookery_kernel::lint::lint(&place).unwrap_or_else(|err| panic!("lint place: {err}"));
    assert!(
        hits.iter()
            .any(|hit| hit.path == "world/places/docks_shop.md" && hit.text == LEDGER),
        "place body was skipped: {hits:?}"
    );
    let _ = fs::remove_dir_all(&place);

    let folded = temp_pack("case");
    append(&folded, "cast/mira.md", "the lamp-oil ledger is short.");
    let hits = spookery_kernel::lint::lint(&folded).unwrap_or_else(|err| panic!("lint case: {err}"));
    assert!(hits.is_empty(), "case-folded body counted as a hit: {hits:?}");
    let _ = fs::remove_dir_all(&folded);
}
