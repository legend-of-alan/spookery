use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::check::{load_checks, CheckDef};
use crate::error::Reason;
use crate::hash::blake2s_hex;
use crate::limits::{load_tick_rules, Budget};
use crate::meter::{self, MeterDef, MeterLive};
use crate::model::Now;
use crate::moves::{load_moves, Audience, MoveDef, RowEffect};

#[derive(Clone, Debug)]
pub struct Canon {
    pub pack_id: String,
    pub title: String,
    pub system: String,
    pub start_place: String,
    pub start_pc: String,
    pub places: BTreeMap<String, PlaceCanon>,
    pub cast: BTreeMap<String, CastCanon>,
    pub facts: BTreeMap<String, FactCanon>,
    pub moves: BTreeMap<String, MoveDef>,
    pub checks: BTreeMap<String, CheckDef>,
    pub meters: BTreeMap<String, MeterDef>,
    pub budget: Budget,
}

#[derive(Clone, Debug)]
pub struct FactCanon {
    pub text: String,
    pub hidden: bool,
    pub owner_id: String,
}

#[derive(Clone, Debug)]
pub struct PlaceCanon {
    pub facts: Vec<String>,
    pub body: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CastKind {
    Npc,
    PcTemplate,
}

#[derive(Clone, Debug)]
pub struct CastCanon {
    pub kind: CastKind,
    pub facts: Vec<String>,
    pub body: String,
    pub tick: Option<TickProfile>,
}

#[derive(Clone, Debug)]
pub struct TickProfile {
    pub eligible: bool,
    pub range: Vec<String>,
    pub stock_rate: BTreeMap<String, BTreeMap<String, i64>>,
    pub flags: Vec<String>,
    pub habits: Vec<Habit>,
}

#[derive(Clone, Debug)]
pub struct Habit {
    pub id: String,
    pub runs_while_empty: bool,
    pub from_hour: Option<i64>,
    pub to_hour: Option<i64>,
    pub clock: Option<String>,
    pub at_least: Option<i64>,
    pub unless_message: Option<String>,
    pub rows: Vec<String>,
}

impl Canon {
    pub fn profile(&self, actor_id: &str) -> Option<&TickProfile> {
        self.cast.get(actor_id).and_then(|cast| cast.tick.as_ref())
    }

    pub fn body_of_fact(&self, fact_id: &str) -> &str {
        let Some(fact) = self.facts.get(fact_id) else {
            return "";
        };
        if let Some(place) = self.places.get(&fact.owner_id) {
            return &place.body;
        }
        if let Some(cast) = self.cast.get(&fact.owner_id) {
            return &cast.body;
        }
        ""
    }

    pub fn needs_roll(&self, move_name: &str) -> bool {
        self.moves
            .get(move_name)
            .is_some_and(|def| def.check.is_some())
    }

    /// Difficulty on a row. A move without a row uses 0.
    pub fn row_difficulty(&self, move_name: &str) -> i64 {
        self.moves
            .get(move_name)
            .and_then(|def| def.row.as_ref())
            .map(|row| row.difficulty)
            .unwrap_or(0)
    }

    /// Check id named by a row. A move without `effects` is not a row.
    pub fn row_check(&self, move_name: &str) -> Option<String> {
        let def = self.moves.get(move_name)?;
        def.row.as_ref()?;
        def.check.clone()
    }
}

pub fn pack_rev(root: &Path) -> Result<String, Reason> {
    let mut files = Vec::new();
    for dir in ["world", "cast", "rules"] {
        collect_files(&root.join(dir), root, &mut files)?;
    }
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut preimage = Vec::new();
    for (rel, bytes) in files {
        preimage.extend(rel.as_bytes());
        preimage.push(0);
        preimage.extend((bytes.len() as u64).to_be_bytes());
        preimage.extend(bytes);
    }
    Ok(format!("blake2s:{}", blake2s_hex(&preimage)))
}

pub fn load_canon(root: &Path) -> Result<Canon, Reason> {
    let pack = read_pack(&root.join("pack.yaml"))?;
    let mut places = BTreeMap::new();
    let mut cast = BTreeMap::new();
    let mut facts = BTreeMap::new();
    let mut place_files = Vec::new();
    collect_files(&root.join("world"), root, &mut place_files)?;
    for (rel, bytes) in place_files {
        if !rel.ends_with(".md") {
            continue;
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| Reason::Schema)?;
        let (yaml, body) = split_frontmatter(text)?;
        let file = load_place(yaml)?;
        for fact in file.facts {
            insert_fact(&mut facts, fact, &file.id)?;
        }
        let ids = fact_ids_of(&facts, &file.id);
        if places
            .insert(
                file.id.clone(),
                PlaceCanon {
                    facts: ids,
                    body: body.to_string(),
                },
            )
            .is_some()
        {
            return Err(Reason::Schema);
        }
    }
    let mut cast_files = Vec::new();
    collect_files(&root.join("cast"), root, &mut cast_files)?;
    for (rel, bytes) in cast_files {
        if !rel.ends_with(".md") {
            continue;
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| Reason::Schema)?;
        let (yaml, body) = split_frontmatter(text)?;
        let file = load_cast(yaml)?;
        for fact in &file.facts {
            insert_fact(
                &mut facts,
                FactFile {
                    id: fact.id.clone(),
                    text: fact.text.clone(),
                    hidden: fact.hidden,
                },
                &file.id,
            )?;
        }
        let tick = match file.tick {
            Some(tick) => Some(validate_tick(tick)?),
            None => None,
        };
        let kind = match file.kind.as_str() {
            "npc" => CastKind::Npc,
            "pc_template" => CastKind::PcTemplate,
            _ => return Err(Reason::Schema),
        };
        if cast
            .insert(
                file.id.clone(),
                CastCanon {
                    kind,
                    facts: fact_ids_of(&facts, &file.id),
                    body: body.to_string(),
                    tick,
                },
            )
            .is_some()
        {
            return Err(Reason::Schema);
        }
    }
    let moves = load_moves(&read_string(&root.join("rules/moves.yaml"))?)?;
    let checks = load_checks(&read_string(&root.join("rules/checks.yaml"))?)?;
    let meters = load_meters(&read_string(&root.join("rules/meters.yaml"))?)?;
    require_known_checks(&moves, &checks)?;
    require_meter_ids(&meters, &checks, &moves)?;
    require_habit_rows(&cast, &moves)?;
    let budget = load_tick_rules(&read_string(&root.join("rules/tick.yaml"))?)?;
    Ok(Canon {
        pack_id: pack.id,
        title: pack.title,
        system: pack.system,
        start_place: pack.start.place,
        start_pc: pack.start.pc,
        places,
        cast,
        facts,
        moves,
        checks,
        meters,
        budget,
    })
}

fn fact_ids_of(facts: &BTreeMap<String, FactCanon>, owner: &str) -> Vec<String> {
    facts
        .iter()
        .filter(|(_, fact)| fact.owner_id == owner)
        .map(|(id, _)| id.clone())
        .collect()
}

fn insert_fact(
    facts: &mut BTreeMap<String, FactCanon>,
    fact: FactFile,
    owner: &str,
) -> Result<(), Reason> {
    if fact.id.is_empty() {
        return Err(Reason::Schema);
    }
    if facts
        .insert(
            fact.id,
            FactCanon {
                text: fact.text,
                hidden: fact.hidden,
                owner_id: owner.to_string(),
            },
        )
        .is_some()
    {
        return Err(Reason::Schema);
    }
    Ok(())
}

fn validate_tick(file: TickFile) -> Result<TickProfile, Reason> {
    Ok(TickProfile {
        eligible: file.eligible,
        range: file.range,
        stock_rate: file.stock_rate,
        flags: file.flags,
        habits: habits_of(file.habits)?,
    })
}

fn habits_of(entries: Vec<HabitFile>) -> Result<Vec<Habit>, Reason> {
    let mut habits = Vec::with_capacity(entries.len());
    let mut seen = BTreeSet::new();
    for entry in entries {
        if entry.id.is_empty() || !seen.insert(entry.id.clone()) || entry.rows.is_empty() {
            return Err(Reason::Schema);
        }
        if entry.rows.iter().any(|row| row.is_empty()) {
            return Err(Reason::Schema);
        }
        let hours = match (entry.from_hour, entry.to_hour) {
            (None, None) => (None, None),
            (Some(from), Some(to)) if (0..24).contains(&from) && (0..24).contains(&to) => {
                (Some(from), Some(to))
            }
            _ => return Err(Reason::Schema),
        };
        let clock = match (entry.clock, entry.at_least) {
            (None, None) => (None, None),
            (Some(id), Some(at_least)) if !id.is_empty() => (Some(id), Some(at_least)),
            _ => return Err(Reason::Schema),
        };
        if entry
            .unless_message
            .as_ref()
            .is_some_and(|id| id.is_empty())
        {
            return Err(Reason::Schema);
        }
        habits.push(Habit {
            id: entry.id,
            runs_while_empty: entry.runs_while_empty,
            from_hour: hours.0,
            to_hour: hours.1,
            clock: clock.0,
            at_least: clock.1,
            unless_message: entry.unless_message,
            rows: entry.rows,
        });
    }
    Ok(habits)
}

pub fn load_meters(text: &str) -> Result<BTreeMap<String, MeterDef>, Reason> {
    crate::format::require_tag(text, crate::format::METERS)?;
    let file: MetersFile = serde_saphyr::from_str(text).map_err(|_| Reason::Schema)?;
    if file.schema != crate::format::METERS {
        return Err(Reason::Schema);
    }
    let mut meters = BTreeMap::new();
    for entry in file.meters {
        if entry.id.is_empty()
            || entry.min > entry.base_cap
            || entry.base_cap > entry.max
            || !(entry.min..=entry.base_cap).contains(&entry.start)
            || meters
                .insert(
                    entry.id,
                    MeterDef {
                        min: entry.min,
                        base_cap: entry.base_cap,
                        max: entry.max,
                        start: entry.start,
                    },
                )
                .is_some()
        {
            return Err(Reason::Schema);
        }
    }
    Ok(meters)
}

/// Unknown ids and pairs `stored` rejects are `SCHEMA`. A defined id missing
/// from a present map is filled from `start` with offset 0.
pub fn bind_meters(canon: &Canon, now: &mut Now) -> Result<(), Reason> {
    for actor in now.actors.values_mut() {
        let live = actor.meters_mut();
        if live.keys().any(|key| !canon.meters.contains_key(key)) {
            return Err(Reason::Schema);
        }
        for (key, meter) in live.iter() {
            let def = &canon.meters[key];
            if !meter::stored(def, meter.base, meter.offset) {
                return Err(Reason::Schema);
            }
        }
        for (id, def) in &canon.meters {
            live.entry(id.clone())
                .or_insert_with(|| MeterLive::at_start(def));
        }
    }
    Ok(())
}

/// An empty id or a duplicate is `SCHEMA`. This does not fill and does not
/// look at meter pairs.
pub fn bind_inventory(now: &Now) -> Result<(), Reason> {
    for actor in now.actors.values() {
        let mut seen = BTreeSet::new();
        for id in actor.inventory() {
            if id.is_empty() || !seen.insert(id.as_str()) {
                return Err(Reason::Schema);
            }
        }
    }
    Ok(())
}

fn require_meter_ids(
    meters: &BTreeMap<String, MeterDef>,
    checks: &BTreeMap<String, CheckDef>,
    moves: &BTreeMap<String, MoveDef>,
) -> Result<(), Reason> {
    for check in checks.values() {
        if !meters.contains_key(&check.stat) {
            return Err(Reason::Schema);
        }
    }
    for def in moves.values() {
        let Some(row) = def.row.as_ref() else {
            continue;
        };
        for effect in row.effects.iter().chain(&row.success).chain(&row.failure) {
            let RowEffect::Meter(write) = effect else {
                continue;
            };
            if !meters.contains_key(&write.key) {
                return Err(Reason::Schema);
            }
        }
    }
    Ok(())
}

fn require_known_checks(
    moves: &BTreeMap<String, MoveDef>,
    checks: &BTreeMap<String, CheckDef>,
) -> Result<(), Reason> {
    for def in moves.values() {
        if let Some(id) = &def.check {
            if !checks.contains_key(id) {
                return Err(Reason::Schema);
            }
        }
    }
    Ok(())
}

fn require_habit_rows(
    cast: &BTreeMap<String, CastCanon>,
    moves: &BTreeMap<String, MoveDef>,
) -> Result<(), Reason> {
    for profile in cast.values().filter_map(|cast| cast.tick.as_ref()) {
        for habit in &profile.habits {
            for name in &habit.rows {
                let Some(row) = moves.get(name).and_then(|def| def.row.as_ref()) else {
                    return Err(Reason::Schema);
                };
                if row.audience != Some(Audience::Habit) {
                    return Err(Reason::Schema);
                }
            }
        }
    }
    Ok(())
}

fn read_pack(path: &Path) -> Result<PackFile, Reason> {
    if !path.is_file() {
        return Err(Reason::NotInSpec);
    }
    load_pack(&read_string(path)?)
}

fn load_pack(text: &str) -> Result<PackFile, Reason> {
    crate::format::require_tag(text, crate::format::PACK)?;
    let file: PackFile = serde_saphyr::from_str(text).map_err(|_| Reason::Schema)?;
    if file.schema != crate::format::PACK || file.id.is_empty() {
        return Err(Reason::Schema);
    }
    Ok(file)
}

fn load_place(yaml: &str) -> Result<PlaceFile, Reason> {
    crate::format::require_tag(yaml, crate::format::PLACE)?;
    let file: PlaceFile = serde_saphyr::from_str(yaml).map_err(|_| Reason::Schema)?;
    if file.schema != crate::format::PLACE || file.id.is_empty() {
        return Err(Reason::Schema);
    }
    Ok(file)
}

fn load_cast(yaml: &str) -> Result<CastFile, Reason> {
    crate::format::require_tag(yaml, crate::format::CAST)?;
    let file: CastFile = serde_saphyr::from_str(yaml).map_err(|_| Reason::Schema)?;
    if file.schema != crate::format::CAST || file.id.is_empty() {
        return Err(Reason::Schema);
    }
    Ok(file)
}

fn read_string(path: &Path) -> Result<String, Reason> {
    fs::read_to_string(path).map_err(|_| {
        if path.is_file() {
            Reason::Schema
        } else {
            Reason::NotInSpec
        }
    })
}

fn collect_files(dir: &Path, root: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<(), Reason> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(dir)
        .map_err(|_| Reason::Schema)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Reason::Schema)?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, root, out)?;
            continue;
        }
        let rel = path.strip_prefix(root).map_err(|_| Reason::Schema)?;
        let rel = rel
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = fs::read(&path).map_err(|_| Reason::Schema)?;
        out.push((rel, bytes));
    }
    Ok(())
}

fn split_frontmatter(text: &str) -> Result<(&str, &str), Reason> {
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or(Reason::Schema)?;
    if let Some(index) = rest.find("\n---\n") {
        return Ok((&rest[..index], &rest[index + 5..]));
    }
    if let Some(index) = rest.find("\n---\r\n") {
        return Ok((&rest[..index], &rest[index + 6..]));
    }
    if let Some(yaml) = rest.strip_suffix("\n---") {
        return Ok((yaml, ""));
    }
    Err(Reason::Schema)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackFile {
    schema: String,
    id: String,
    title: String,
    system: String,
    start: PackStart,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackStart {
    place: String,
    pc: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceFile {
    schema: String,
    id: String,
    facts: Vec<FactFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FactFile {
    id: String,
    text: String,
    hidden: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CastFile {
    schema: String,
    id: String,
    kind: String,
    facts: Vec<FactFile>,
    #[serde(default)]
    tick: Option<TickFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TickFile {
    eligible: bool,
    #[serde(default)]
    range: Vec<String>,
    #[serde(default)]
    stock_rate: BTreeMap<String, BTreeMap<String, i64>>,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    habits: Vec<HabitFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HabitFile {
    id: String,
    runs_while_empty: bool,
    #[serde(default)]
    from_hour: Option<i64>,
    #[serde(default)]
    to_hour: Option<i64>,
    #[serde(default)]
    clock: Option<String>,
    #[serde(default)]
    at_least: Option<i64>,
    #[serde(default)]
    unless_message: Option<String>,
    rows: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MetersFile {
    schema: String,
    meters: Vec<MeterEntryFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MeterEntryFile {
    id: String,
    min: i64,
    base_cap: i64,
    max: i64,
    start: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Reason;

    #[test]
    fn docks_pack_rev_matches_now_yaml() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        let rev = pack_rev(&root).unwrap();
        let text = fs::read_to_string(root.join("state/now.yaml")).unwrap();
        let line = text
            .lines()
            .find(|line| line.starts_with("pack_rev:"))
            .unwrap();
        assert_eq!(line, format!("pack_rev: {rev}"));
    }

    #[test]
    fn load_meters_rejects_a_broken_inequality_a_duplicate_id_and_an_unknown_key() {
        let broken = "\
schema: meters/v0
meters:
  - id: grit
    min: 2
    base_cap: 1
    max: 3
    start: 1
";
        assert_eq!(load_meters(broken).unwrap_err(), Reason::Schema);
        let duplicate = "\
schema: meters/v0
meters:
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 1
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 0
";
        assert_eq!(load_meters(duplicate).unwrap_err(), Reason::Schema);
        let unknown = "\
schema: meters/v0
meters:
  - id: grit
    min: 0
    base_cap: 1
    max: 1
    start: 1
    kind: stat
";
        assert_eq!(load_meters(unknown).unwrap_err(), Reason::Schema);
    }

    fn rejects(load: fn(&str) -> Result<(), Reason>, good: &str, old: &str, body: &str) {
        let text = |tag: &str| format!("schema: {tag}\n{body}");
        assert!(load(&text(good)).is_ok());
        assert_eq!(load(&text(old)).unwrap_err(), Reason::Schema);
        assert_eq!(load(&text("bogus")).unwrap_err(), Reason::Schema);
    }

    #[test]
    fn load_place_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_place(text).map(|_| ()),
            "place",
            concat!("place/", "v0"),
            "id: gate\nfacts: []\n",
        );
    }

    #[test]
    fn load_cast_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_cast(text).map(|_| ()),
            "cast",
            concat!("cast/", "v0"),
            "id: mira\nkind: npc\nfacts: []\n",
        );
    }

    #[test]
    fn load_pack_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_pack(text).map(|_| ()),
            "pack",
            concat!("pack/", "v0"),
            "id: ex\ntitle: Ex\nsystem: ex\nstart:\n  place: gate\n  pc: pc_rowan\n",
        );
    }

    #[test]
    fn load_meters_rejects_an_unknown_tag_and_the_old_versioned_tag() {
        rejects(
            |text| load_meters(text).map(|_| ()),
            "meters",
            concat!("meters/", "v0"),
            "meters: []\n",
        );
    }
}
