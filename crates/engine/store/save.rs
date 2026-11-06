//! Read, write and migrate the save.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_saphyr::SerializerOptions;

use crate::canon::Canon;
use crate::diagnostic::{check_tag, error, locate_nth, parse_file, schema_error, Diagnostic};
use crate::model::meter::MeterLive;
use crate::model::modifier::{self, ModifierLive};
use crate::model::{exit_parts, sort_unique, Act, ActorKind, ActorLive, Note, PlaceLive};
use crate::model::{State, StoryLive};
use crate::rules::effect::Holder;
use crate::store::records::{AddedMeter, DroppedMeter};
use crate::Reason;

/// `save.yaml` as it stands on disk, where each record holds its own meters and modifiers. A
/// `State` keeps those in stores instead, so `parse` and `emit` move them between the two.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveFile {
    schema: String,
    world_id: String,
    pack_rev: String,
    /// Engine rules version. Missing means 0, which is not the current version.
    #[serde(default)]
    rules_rev: u64,
    state_rev: u64,
    round_index: u64,
    world_time: u64,
    last_round_at: String,
    places: BTreeMap<String, PlaceSave>,
    actors: BTreeMap<String, ActorSave>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    stories: BTreeMap<String, StorySave>,
    /// The meters `world/meters.yaml` declares, held once by the world.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    world_meters: BTreeMap<String, MeterLive>,
    notes: Vec<Note>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    flags: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaceSave {
    items: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    meters: BTreeMap<String, MeterLive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    flags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    open: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StorySave {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    meters: BTreeMap<String, MeterLive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    flags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActorSave {
    kind: ActorKind,
    place: String,
    flags: Vec<String>,
    meters: BTreeMap<String, MeterLive>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    modifiers: BTreeMap<String, ModifierLive>,
    inventory: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wielded: Option<String>,
    known_facts: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    visited: BTreeMap<String, Vec<String>>,
    act: Act,
}

impl From<SaveFile> for State {
    /// Each record's meters and modifiers go to the stores, under it.
    fn from(file: SaveFile) -> Self {
        let mut state = State {
            schema: file.schema,
            world_id: file.world_id,
            pack_rev: file.pack_rev,
            rules_rev: file.rules_rev,
            state_rev: file.state_rev,
            round_index: file.round_index,
            world_time: file.world_time,
            last_round_at: file.last_round_at,
            places: BTreeMap::new(),
            actors: BTreeMap::new(),
            stories: BTreeMap::new(),
            meters: Default::default(),
            modifiers: Default::default(),
            notes: file.notes.into(),
            flags: file.flags.into(),
            ledger: Default::default(),
        };
        let mut meters = |holder: Holder, held: BTreeMap<String, MeterLive>| {
            for (key, live) in held {
                state.meters.insert(holder.clone(), key, live);
            }
        };
        meters(Holder::World, file.world_meters);
        let mut places = BTreeMap::new();
        for (id, place) in file.places {
            meters(Holder::Place(id.clone()), place.meters);
            let PlaceSave {
                items, flags, open, ..
            } = place;
            places.insert(id, PlaceLive { items, flags, open }.into());
        }
        let mut stories = BTreeMap::new();
        for (id, story) in file.stories {
            meters(Holder::Story(id.clone()), story.meters);
            stories.insert(id, StoryLive { flags: story.flags }.into());
        }
        let mut actors = BTreeMap::new();
        let mut modifiers = Vec::new();
        for (id, actor) in file.actors {
            let holder = Holder::Actor(id.clone());
            meters(holder.clone(), actor.meters);
            modifiers.extend(
                actor
                    .modifiers
                    .into_iter()
                    .map(|(key, live)| (holder.clone(), key, live)),
            );
            let record = ActorLive {
                kind: actor.kind,
                place: actor.place,
                flags: actor.flags,
                inventory: actor.inventory,
                wielded: actor.wielded,
                known_facts: actor.known_facts,
                visited: actor.visited,
                act: actor.act,
            };
            actors.insert(id, record.into());
        }
        for (holder, key, live) in modifiers {
            state.modifiers.insert(holder, key, live);
        }
        state.places = places;
        state.stories = stories;
        state.actors = actors;
        state
    }
}

impl From<&State> for SaveFile {
    /// Each meter and modifier goes back into its holder's record.
    fn from(state: &State) -> Self {
        let mut file = SaveFile {
            schema: state.schema.clone(),
            world_id: state.world_id.clone(),
            pack_rev: state.pack_rev.clone(),
            rules_rev: state.rules_rev,
            state_rev: state.state_rev,
            round_index: state.round_index,
            world_time: state.world_time,
            last_round_at: state.last_round_at.clone(),
            places: BTreeMap::new(),
            actors: BTreeMap::new(),
            stories: BTreeMap::new(),
            world_meters: BTreeMap::new(),
            notes: state.notes.to_vec(),
            flags: state.flags.to_vec(),
        };
        for (id, place) in &state.places {
            let PlaceLive { items, flags, open } = &**place;
            let (items, flags, open) = (items.clone(), flags.clone(), open.clone());
            let place = PlaceSave {
                items,
                flags,
                open,
                ..PlaceSave::default()
            };
            file.places.insert(id.clone(), place);
        }
        for (id, story) in &state.stories {
            let flags = story.flags.clone();
            file.stories.insert(
                id.clone(),
                StorySave {
                    flags,
                    ..StorySave::default()
                },
            );
        }
        for (id, actor) in &state.actors {
            let actor = ActorSave {
                kind: actor.kind,
                place: actor.place.clone(),
                flags: actor.flags.clone(),
                meters: BTreeMap::new(),
                modifiers: BTreeMap::new(),
                inventory: actor.inventory.clone(),
                wielded: actor.wielded.clone(),
                known_facts: actor.known_facts.clone(),
                visited: actor.visited.clone(),
                act: actor.act.clone(),
            };
            file.actors.insert(id.clone(), actor);
        }
        for (holder, key, live) in state.meters.iter() {
            let meters = match holder {
                Holder::Actor(id) => file.actors.get_mut(id).map(|actor| &mut actor.meters),
                Holder::Place(id) => Some(&mut file.places.entry(id.clone()).or_default().meters),
                Holder::Story(id) => Some(&mut file.stories.entry(id.clone()).or_default().meters),
                Holder::World => Some(&mut file.world_meters),
                Holder::Exit(..) => None,
            };
            let meters = meters.expect("a meter's holder has a record that can hold it");
            meters.insert(key.clone(), live.clone());
        }
        for (holder, key, live) in state.modifiers.iter() {
            let Holder::Actor(id) = holder else {
                unreachable!("only an actor holds modifiers");
            };
            let actor = file
                .actors
                .get_mut(id)
                .expect("a modifier's actor has a record");
            actor.modifiers.insert(key.clone(), live.clone());
        }
        file
    }
}

pub(crate) fn emit_options() -> SerializerOptions {
    serde_saphyr::ser_options! {
        empty_as_braces: true,
        indent_step: 2,
        compact_list_indent: false,
        anchor_generator: None,
        min_fold_chars: usize::MAX,
        folded_wrap_chars: usize::MAX,
        tagged_enums: false,
        prefer_block_scalars: false,
        quote_all: false,
        yaml_12: false,
    }
}

pub fn parse(text: &str) -> Result<State, Vec<Diagnostic>> {
    const FILE: &str = "state/save.yaml";
    if let Some(diag) = check_tag(text, crate::diagnostic::SAVE_SCHEMA, FILE, 0) {
        return Err(vec![diag]);
    }
    // Check the rules version before the full shape, so a save from older rules
    // (for example one that still carries `presence`) is refused as RULES_REV_MISMATCH.
    #[derive(Deserialize)]
    struct RulesProbe {
        #[serde(default)]
        rules_rev: u64,
    }
    if let Ok(probe) = serde_saphyr::from_str::<RulesProbe>(text) {
        if super::require_rules_rev(probe.rules_rev).is_err() {
            return Err(vec![rules_rev_diagnostic(text)]);
        }
    }
    let state: State = parse_file::<SaveFile>(FILE, text, 0)?.into();
    let diags = check_actor_places(&state);
    if diags.is_empty() {
        Ok(state)
    } else {
        Err(diags)
    }
}

pub fn emit(state: &State) -> Result<String, Reason> {
    let mut canonical = state.clone();
    canonical.schema = crate::diagnostic::SAVE_SCHEMA.to_string();
    canonical.rules_rev = super::RULES_REV;
    normalize(&mut canonical);
    let file = SaveFile::from(&canonical);
    let text =
        serde_saphyr::to_string_with_options(&file, emit_options()).map_err(|_| Reason::Schema)?;
    Ok(one_trailing_lf(text))
}

fn rules_rev_diagnostic(text: &str) -> Diagnostic {
    let mut diag = error(
        "state/save.yaml",
        Reason::RulesRevMismatch,
        "this save was written by different rules",
        "start a fresh save",
    )
    .at_path("rules_rev");
    if let Some((line, column)) = locate_nth(text, "rules_rev:", 0) {
        diag = diag.at_line(line, Some(column));
    }
    diag
}

fn one_trailing_lf(mut text: String) -> String {
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    text
}

pub fn normalize(state: &mut State) {
    let notes = state.notes.frontier_mut();
    notes.sort_by(|a, b| a.id.cmp(&b.id));
    for note in notes {
        sort_unique(&mut note.to);
        sort_unique(&mut note.fact_ids);
    }
    for actor in state.actors.values_mut() {
        let actor = actor.frontier_mut();
        sort_unique(&mut actor.flags);
        sort_unique(&mut actor.known_facts);
    }
    for place in state.places.values_mut() {
        let place = place.frontier_mut();
        sort_unique(&mut place.flags);
        sort_unique(&mut place.open);
    }
    for story in state.stories.values_mut() {
        sort_unique(&mut story.frontier_mut().flags);
    }
    sort_unique(state.flags.frontier_mut());
}

fn check_actor_places(state: &State) -> Vec<Diagnostic> {
    const FILE: &str = "state/save.yaml";
    let mut diags = Vec::new();
    for (id, actor) in &state.actors {
        // A walker stands on an exit of a place until they arrive.
        let place = exit_parts(&actor.place).map_or(actor.place.as_str(), |(place, _)| place);
        if state.places.contains_key(place) {
            continue;
        }
        diags.push(
            schema_error(
                FILE,
                &format!(
                    "actor '{id}' is in place '{}', which is not in places",
                    actor.place
                ),
                "add the place or move the actor",
            )
            .at_path(format!("actors.{id}.place")),
        );
    }
    diags
}

/// Meters the pack gives a holder that the holder lacks, each at its `start`.
pub(crate) fn missing_meters(canon: &Canon, state: &State) -> Vec<AddedMeter> {
    let mut added = Vec::new();
    let lacks = |holder: &Holder, id: &str| state.meters.get(holder, id).is_none();
    for (id, def) in &canon.meters {
        let lacking = match &def.carrier {
            Some(holder) if !def.is_actor_meter() => lacks(holder, id),
            _ => state.actors.keys().any(|actor| {
                let actor = Holder::Actor(actor.clone());
                def.is_on(&actor) && lacks(&actor, id)
            }),
        };
        if lacking {
            added.push(AddedMeter::new(id, def));
        }
    }
    for (id, def) in &canon.world_meters {
        if lacks(&Holder::World, id) {
            added.push(AddedMeter::new(id, def));
        }
    }
    added
}

/// Meters an actor, place, story or the world lists that the pack no longer gives it.
pub(crate) fn stale_meters(canon: &Canon, state: &State) -> Vec<DroppedMeter> {
    let stale = state.meters.iter();
    let stale = stale.filter(|(carrier, id, _)| canon.meter_def(carrier, id).is_none());
    let dropped = stale.map(|(carrier, id, _)| DroppedMeter {
        id: id.clone(),
        carrier: carrier.clone(),
    });
    dropped.collect()
}

/// Modifiers some actor holds that the pack no longer defines, or now defines on another meter.
pub(crate) fn stale_modifiers(canon: &Canon, state: &State) -> Vec<String> {
    let mut stale: Vec<String> = state
        .modifiers
        .iter()
        .filter(|(_, id, live)| !modifier::is_defined(&canon.modifiers, id, live))
        .map(|(_, id, _)| id.clone())
        .collect();
    sort_unique(&mut stale);
    stale
}

/// Migrate a save: give each holder the `meters` it lacks, at their `start`, drop the
/// `dropped_modifiers` from every actor and each `dropped_meters` from its holder, then stamp
/// `pack_rev` and `state_rev`. Live migrate and replay both call this. It does not touch the disk.
pub fn migrate(
    mut state: State,
    pack_rev: &str,
    state_rev: u64,
    meters: &[AddedMeter],
    dropped_modifiers: &[String],
    dropped_meters: &[DroppedMeter],
) -> State {
    for meter in meters {
        let start = MeterLive {
            base: meter.start,
            offset: 0,
        };
        let holders: Vec<Holder> = match &meter.carrier {
            Some(Holder::Place(id)) => {
                state.places.entry(id.clone()).or_default();
                vec![Holder::Place(id.clone())]
            }
            Some(Holder::Story(id)) => {
                state.stories.entry(id.clone()).or_default();
                vec![Holder::Story(id.clone())]
            }
            Some(holder @ Holder::World) => vec![holder.clone()],
            carrier => (state.actors.keys())
                .map(|id| Holder::Actor(id.clone()))
                .filter(|actor| carrier.as_ref().is_none_or(|carrier| carrier == actor))
                .collect(),
        };
        for holder in holders {
            if state.meters.get(&holder, &meter.id).is_none() {
                state.meters.insert(holder, meter.id.clone(), start.clone());
            }
        }
    }
    (state.modifiers).retain(|_, id, _| !dropped_modifiers.iter().any(|dropped| dropped == id));
    for meter in dropped_meters {
        state.meters.remove(&meter.carrier, &meter.id);
    }
    state.pack_rev = pack_rev.to_string();
    state.state_rev = state_rev;
    normalize(&mut state);
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/save.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let state = parse(&original).unwrap();
        let emitted = emit(&state).unwrap();
        assert_eq!(emitted, original);
        assert_eq!(parse(&emitted).unwrap(), state);
    }

    fn save_text(tag: &str) -> String {
        format!(
            "\
schema: {tag}
world_id: ex
pack_rev: blake2s:00
rules_rev: {rules}
state_rev: 0
round_index: 0
world_time: 0
last_round_at: \"2026-01-01T00:00:00Z\"
places:
  gate:
    items: {{}}
actors: {{}}
notes: []
",
            rules = crate::store::RULES_REV,
        )
    }

    #[test]
    fn emit_writes_the_save_tag() {
        let mut state = parse(&save_text("save")).unwrap();
        state.schema = concat!("save/", "v0").to_string();
        let emitted = emit(&state).unwrap();
        assert!(emitted.starts_with("schema: save\n"), "{emitted}");
        assert_eq!(parse(&emitted).unwrap().schema, "save");
    }
}
