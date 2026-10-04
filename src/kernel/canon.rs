use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use blake2::{Blake2s256, Digest};
use serde::Deserialize;

use crate::check::{load_checks, CheckDef};
use crate::error::Reason;
use crate::limits::{load_tick_rules, Budget};
use crate::moves::{load_moves, Audience, MoveDef};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Named,
    Major,
    #[default]
    Minor,
}

#[derive(Clone, Debug)]
pub struct TickProfile {
    pub eligible: bool,
    pub tier: Tier,
    pub plot_relevant: bool,
    pub range: Vec<String>,
    pub schedule: Vec<ScheduleRow>,
    pub stock_rate: BTreeMap<String, BTreeMap<String, i64>>,
    pub needs: BTreeMap<String, Bound>,
    pub resources: BTreeMap<String, Bound>,
    pub flags: Vec<String>,
    pub work: Option<WorkBlock>,
    pub notes: Vec<NoteDef>,
    pub agenda: Vec<AgendaEntry>,
}

impl TickProfile {
    pub fn on_graph(&self, place: &str) -> bool {
        self.schedule.iter().any(|row| row.place == place)
    }

    pub fn winning_row(&self, hour: i64) -> Option<&ScheduleRow> {
        self.schedule.iter().find(|row| hour_in(row.start, row.end, hour))
    }
}

#[derive(Clone, Debug)]
pub struct ScheduleRow {
    pub start: i64,
    pub end: i64,
    pub place: String,
    pub activity: Option<String>,
    pub rest_delta: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct Bound {
    pub min: Option<i64>,
    pub max: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct WorkBlock {
    pub needs: Vec<WorkDelta>,
    pub resources: Vec<WorkDelta>,
}

#[derive(Clone, Debug)]
pub struct WorkDelta {
    pub key: String,
    pub delta: i64,
}

#[derive(Clone, Debug)]
pub struct NoteDef {
    pub id: String,
    pub due_clock: String,
    pub due_value: i64,
    pub to: Vec<String>,
    pub place: String,
    pub fact_ids: Vec<String>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct AgendaEntry {
    pub row: String,
    pub pause: Option<PauseLane>,
    pub schedule: Option<ScheduleGate>,
    pub activity: Option<String>,
    pub when_stock: bool,
    pub when_drift: bool,
    pub when_rumor: bool,
    pub when_note: bool,
    pub when_clock: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PauseLane {
    NamedPlotClocks,
    SubsistenceClocks,
    RelationshipDrift,
    Travel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleGate {
    Here,
    Away,
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
}

pub fn hour_in(start: i64, end: i64, hour: i64) -> bool {
    if start < end {
        start <= hour && hour < end
    } else if start > end {
        hour >= start || hour < end
    } else {
        false
    }
}

pub fn blake2s_256(bytes: &[u8]) -> [u8; 32] {
    let digest = Blake2s256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn blake2s_hex(bytes: &[u8]) -> String {
    hex_encode(&blake2s_256(bytes))
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
        let file: PlaceFile = serde_saphyr::from_str(yaml).map_err(|_| Reason::Schema)?;
        if file.schema != "place/v0" || file.id.is_empty() {
            return Err(Reason::Schema);
        }
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
        let file: CastFile = serde_saphyr::from_str(yaml).map_err(|_| Reason::Schema)?;
        if file.schema != "cast/v0" || file.id.is_empty() {
            return Err(Reason::Schema);
        }
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
    require_known_checks(&moves, &checks)?;
    require_agenda_rows(&cast, &moves)?;
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

fn insert_fact(facts: &mut BTreeMap<String, FactCanon>, fact: FactFile, owner: &str) -> Result<(), Reason> {
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
    for row in &file.schedule {
        if !(0..=24).contains(&row.start) || !(0..=24).contains(&row.end) {
            return Err(Reason::Schema);
        }
        match row.activity.as_deref() {
            None => {}
            Some("work") => {}
            Some("rest") => {
                if row.rest_delta.is_none() {
                    return Err(Reason::Schema);
                }
            }
            Some(_) => return Err(Reason::Schema),
        }
    }
    let mut seen_notes = BTreeSet::new();
    for note in &file.notes {
        if note.id.is_empty() || !seen_notes.insert(note.id.clone()) {
            return Err(Reason::Schema);
        }
    }
    Ok(TickProfile {
        eligible: file.eligible,
        tier: file.tier,
        plot_relevant: file.plot_relevant,
        range: file.range,
        schedule: file
            .schedule
            .into_iter()
            .map(|row| ScheduleRow {
                start: row.start,
                end: row.end,
                place: row.place,
                activity: row.activity,
                rest_delta: row.rest_delta,
            })
            .collect(),
        stock_rate: file.stock_rate,
        needs: file
            .needs
            .into_iter()
            .map(|(key, bound)| {
                (
                    key,
                    Bound {
                        min: bound.min,
                        max: bound.max,
                    },
                )
            })
            .collect(),
        resources: file
            .resources
            .into_iter()
            .map(|(key, bound)| {
                (
                    key,
                    Bound {
                        min: bound.min,
                        max: bound.max,
                    },
                )
            })
            .collect(),
        flags: file.flags,
        work: file.work.map(|work| WorkBlock {
            needs: work
                .needs
                .into_iter()
                .map(|item| WorkDelta {
                    key: item.key,
                    delta: item.delta,
                })
                .collect(),
            resources: work
                .resources
                .into_iter()
                .map(|item| WorkDelta {
                    key: item.key,
                    delta: item.delta,
                })
                .collect(),
        }),
        notes: file
            .notes
            .into_iter()
            .map(|note| NoteDef {
                id: note.id,
                due_clock: note.due_clock,
                due_value: note.due_value,
                to: note.to,
                place: note.place,
                fact_ids: note.fact_ids,
                text: note.text,
            })
            .collect(),
        agenda: agenda_of(file.agenda)?,
    })
}

fn agenda_of(entries: Vec<AgendaFile>) -> Result<Vec<AgendaEntry>, Reason> {
    let mut agenda = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.row.is_empty() {
            return Err(Reason::Schema);
        }
        if entry.activity.is_some() && entry.schedule.as_deref() != Some("here") {
            return Err(Reason::Schema);
        }
        let pause = match entry.pause.as_deref() {
            None => None,
            Some("named_plot_clocks") => Some(PauseLane::NamedPlotClocks),
            Some("subsistence_clocks") => Some(PauseLane::SubsistenceClocks),
            Some("relationship_drift") => Some(PauseLane::RelationshipDrift),
            Some("travel") => Some(PauseLane::Travel),
            Some(_) => return Err(Reason::Schema),
        };
        let schedule = match entry.schedule.as_deref() {
            None => None,
            Some("here") => Some(ScheduleGate::Here),
            Some("away") => Some(ScheduleGate::Away),
            Some(_) => return Err(Reason::Schema),
        };
        agenda.push(AgendaEntry {
            row: entry.row,
            pause,
            schedule,
            activity: entry.activity,
            when_stock: entry.when_stock,
            when_drift: entry.when_drift,
            when_rumor: entry.when_rumor,
            when_note: entry.when_note,
            when_clock: entry.when_clock,
        });
    }
    Ok(agenda)
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

fn require_agenda_rows(
    cast: &BTreeMap<String, CastCanon>,
    moves: &BTreeMap<String, MoveDef>,
) -> Result<(), Reason> {
    for profile in cast.values().filter_map(|cast| cast.tick.as_ref()) {
        for entry in &profile.agenda {
            let Some(row) = moves.get(&entry.row).and_then(|def| def.row.as_ref()) else {
                return Err(Reason::Schema);
            };
            if row.audience != Some(Audience::Agenda) {
                return Err(Reason::Schema);
            }
        }
    }
    Ok(())
}

fn read_pack(path: &Path) -> Result<PackFile, Reason> {
    if !path.is_file() {
        return Err(Reason::NotInSpec);
    }
    let file: PackFile = serde_saphyr::from_str(&read_string(path)?).map_err(|_| Reason::Schema)?;
    if file.schema != "pack/v0" || file.id.is_empty() {
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
    let mut entries = fs::read_dir(dir).map_err(|_| Reason::Schema)?.collect::<Result<Vec<_>, _>>().map_err(|_| Reason::Schema)?;
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

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
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
    tier: Tier,
    #[serde(default)]
    plot_relevant: bool,
    #[serde(default)]
    range: Vec<String>,
    #[serde(default)]
    schedule: Vec<ScheduleFile>,
    #[serde(default)]
    stock_rate: BTreeMap<String, BTreeMap<String, i64>>,
    #[serde(default)]
    needs: BTreeMap<String, BoundFile>,
    #[serde(default)]
    resources: BTreeMap<String, BoundFile>,
    #[serde(default)]
    flags: Vec<String>,
    #[serde(default)]
    work: Option<WorkFile>,
    #[serde(default)]
    notes: Vec<NoteFile>,
    #[serde(default)]
    agenda: Vec<AgendaFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgendaFile {
    row: String,
    #[serde(default)]
    pause: Option<String>,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default)]
    activity: Option<String>,
    #[serde(default)]
    when_stock: bool,
    #[serde(default)]
    when_drift: bool,
    #[serde(default)]
    when_rumor: bool,
    #[serde(default)]
    when_note: bool,
    #[serde(default)]
    when_clock: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScheduleFile {
    start: i64,
    end: i64,
    place: String,
    #[serde(default)]
    activity: Option<String>,
    #[serde(default)]
    rest_delta: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoundFile {
    #[serde(default)]
    min: Option<i64>,
    #[serde(default)]
    max: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkFile {
    #[serde(default)]
    needs: Vec<DeltaFile>,
    #[serde(default)]
    resources: Vec<DeltaFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeltaFile {
    key: String,
    delta: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoteFile {
    id: String,
    due_clock: String,
    due_value: i64,
    to: Vec<String>,
    place: String,
    fact_ids: Vec<String>,
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
