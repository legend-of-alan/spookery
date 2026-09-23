use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::Canon;
use crate::error::Code;

pub fn load_moves(text: &str) -> Result<BTreeMap<String, Option<String>>, Code> {
    let file: MovesFile = serde_saphyr::from_str(text).map_err(|_| Code::Schema)?;
    if file.schema != "moves/v0" {
        return Err(Code::Schema);
    }
    let mut moves = BTreeMap::new();
    for entry in file.moves {
        if entry.name.is_empty() || moves.insert(entry.name, entry.check).is_some() {
            return Err(Code::Schema);
        }
    }
    Ok(moves)
}

pub fn needs_roll(canon: &Canon, move_name: &str) -> bool {
    matches!(canon.moves.get(move_name), Some(Some(_)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MovesFile {
    schema: String,
    moves: Vec<MoveEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveEntry {
    name: String,
    #[serde(default)]
    check: Option<String>,
}
