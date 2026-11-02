//! The engine's modules depend one way, `round → store → canon`: no production line in `store`
//! names `crate::round`, and none in `canon` names `crate::store`.

use std::fs;
use std::path::Path;

#[test]
fn store_and_canon_never_import_back() {
    let engine = Path::new(env!("CARGO_MANIFEST_DIR")).join("../engine");
    let mut back = Vec::new();
    for (module, above) in [("store", "crate::round"), ("canon", "crate::store")] {
        for entry in fs::read_dir(engine.join(module)).unwrap() {
            let path = entry.unwrap().path();
            let text = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            // Production is the lines before the `#[cfg(test)]` that opens `mod tests`.
            let tests = lines
                .windows(2)
                .position(|pair| pair == ["#[cfg(test)]", "mod tests {"]);
            for (index, line) in lines[..tests.unwrap_or(lines.len())].iter().enumerate() {
                if line.contains(above) {
                    back.push(format!("{}:{}: {line}", path.display(), index + 1));
                }
            }
        }
    }
    assert!(back.is_empty(), "imports upward:\n{}", back.join("\n"));
}
