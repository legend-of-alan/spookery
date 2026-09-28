use std::collections::BTreeMap;

use serde::Deserialize;

use crate::canon::Canon;
use crate::error::Code;
use crate::model::{PcSheet, ViewerId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckDef {
    pub dice_count: u32,
    pub dice_sides: u32,
    pub stat: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckOut {
    pub faces: Vec<u32>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub success: bool,
}

/// A check the tick resolved. Not stored in `Now` or the tick JSONL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckNotice {
    Rolled { actor_id: String, out: CheckOut },
    Refused { actor_id: String, code: Code },
}

impl CheckNotice {
    pub fn actor_id(&self) -> &str {
        match self {
            CheckNotice::Rolled { actor_id, .. } | CheckNotice::Refused { actor_id, .. } => actor_id,
        }
    }
}

pub fn load_checks(text: &str) -> Result<BTreeMap<String, CheckDef>, Code> {
    let file: ChecksFile = serde_saphyr::from_str(text).map_err(|_| Code::Schema)?;
    if file.schema != "checks/v0" {
        return Err(Code::Schema);
    }
    let mut checks = BTreeMap::new();
    for entry in file.checks {
        if entry.id.is_empty()
            || entry.dice.count == 0
            || entry.dice.sides == 0
            || checks
                .insert(
                    entry.id,
                    CheckDef {
                        dice_count: entry.dice.count,
                        dice_sides: entry.dice.sides,
                        stat: entry.stat,
                    },
                )
                .is_some()
        {
            return Err(Code::Schema);
        }
    }
    Ok(checks)
}

pub fn die_face(rng: &[u8], n: usize, sides: u32) -> Result<u32, Code> {
    if sides == 0 {
        return Err(Code::Schema);
    }
    let start = n.checked_mul(4).ok_or(Code::NotInSpec)?;
    let end = start.checked_add(4).ok_or(Code::NotInSpec)?;
    if end > rng.len() || end > 32 {
        return Err(Code::NotInSpec);
    }
    let word = u32::from_le_bytes(rng[start..end].try_into().map_err(|_| Code::NotInSpec)?);
    Ok((word % sides) + 1)
}

/// NPC viewers return `UNKNOWN_KEY` and do not advance `die_cursor`.
pub fn resolve_check(
    canon: &Canon,
    sheets: &BTreeMap<String, PcSheet>,
    viewer: &ViewerId,
    check_id: &str,
    target: i64,
    rng: &[u8],
    die_cursor: &mut usize,
) -> Result<CheckOut, Code> {
    let check = canon.checks.get(check_id).ok_or(Code::UnknownKey)?;
    let ViewerId::Player(id) = viewer else {
        return Err(Code::UnknownKey);
    };
    let sheet = sheets.get(id).ok_or(Code::UnknownKey)?;
    let stat = sheet.stats.get(&check.stat).copied().ok_or(Code::UnknownKey)?;
    let mut faces = Vec::with_capacity(check.dice_count as usize);
    for _ in 0..check.dice_count {
        let face = die_face(rng, *die_cursor, check.dice_sides)?;
        *die_cursor += 1;
        faces.push(face);
    }
    let total = faces.iter().map(|face| i64::from(*face)).sum::<i64>() + stat;
    Ok(CheckOut {
        faces,
        stat,
        total,
        target,
        success: total >= target,
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksFile {
    schema: String,
    checks: Vec<CheckEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckEntry {
    id: String,
    dice: Dice,
    stat: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dice {
    count: u32,
    sides: u32,
}
