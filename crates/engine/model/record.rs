//! The records a round writes whole, the pre-image a write keeps, and the keys a write changes.
//! Connects `Draft` to `State`: a write names a record by its id, and the id knows where the
//! record sits and which keys its fields answer for.

use std::collections::{BTreeMap, BTreeSet};
use std::mem::discriminant;

use crate::canon::Canon;
use crate::model::key::{Change, StateKey, StateValue};
use crate::model::meter::{MeterLive, Meters};
use crate::model::{count_of, find_note, is_set, ActorLive, Note, PlaceLive, State, StoryLive};
use crate::rules::effect::Holder;
use crate::Reason;

/// Names one record of the save and knows its layout: where it sits, how it comes to be, and
/// which keys its fields answer for. The record type follows from the id type, so a write
/// cannot pair an actor's id with a place's record.
pub(crate) trait RecordId {
    type Record: Clone + PartialEq;

    /// The record as it is now, or `None` when it has no live record yet.
    fn get<'s>(&self, state: &'s State) -> Option<&'s Self::Record>;

    /// The record to write. A place or story comes to be on its first write; an actor, the
    /// world and the notes are always there.
    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut Self::Record, Reason>;

    /// `before` as the undo log keeps it, under this id.
    fn pre_image(&self, before: Option<Self::Record>) -> PreImage;

    /// Note every key of this record whose stored fields or value differ between `before` and
    /// `after`. `None` is a record that was not there yet. Each key names the field it reads
    /// once, here; how that field reads as a value is the field type's `Into<StateValue>`.
    fn diff(
        &self,
        canon: &Canon,
        before: Option<&Self::Record>,
        after: &Self::Record,
        out: &mut Diff,
    );
}

/// What one comparison of a record found: the keys whose stored fields changed, which the
/// budget counts, and the keys whose value changed, which the watches and seats hear.
#[derive(Default)]
pub(crate) struct Diff {
    pub(crate) spent: Vec<StateKey>,
    pub(crate) changes: Vec<Change>,
}

impl Diff {
    /// A key whose value is its stored field: it changes only when the field does. `key_of`
    /// names the key from `name`, the id or name its last part is.
    fn stored<T: PartialEq + Into<StateValue>>(
        &mut self,
        key_of: impl FnOnce(String) -> StateKey,
        name: &str,
        before: T,
        after: T,
    ) {
        if before != after {
            self.note(key_of, name, true, before.into(), after.into());
        }
    }

    /// A key whose stored fields and value can move apart: a meter's value is read through the
    /// pack and moves with the holder's modifiers; a busy actor's stores the action under way
    /// too. The key is built only when one of the two differs.
    fn note(
        &mut self,
        key_of: impl FnOnce(String) -> StateKey,
        name: &str,
        stored_changed: bool,
        before: StateValue,
        after: StateValue,
    ) {
        if !stored_changed && before == after {
            return;
        }
        let key = key_of(name.to_string());
        if stored_changed {
            self.spent.push(key.clone());
        }
        if before != after {
            self.changes.push(Change { key, before, after });
        }
    }
}

pub(crate) struct ActorId<'a>(pub &'a str);
pub(crate) struct PlaceId<'a>(pub &'a str);
pub(crate) struct StoryId<'a>(pub &'a str);
/// The meters `world/meters.yaml` declares, held once by the world.
pub(crate) struct WorldMetersId;
pub(crate) struct WorldFlagsId;
pub(crate) struct NotesId;

/// One record as it was just before a write, under its id. The undo log is a list of these,
/// oldest first; a rollback puts them back newest first. `None` is a record that was not there.
pub(crate) enum PreImage {
    Actor(String, Option<Box<ActorLive>>),
    Place(String, Option<PlaceLive>),
    Story(String, Option<StoryLive>),
    WorldMeters(BTreeMap<String, MeterLive>),
    WorldFlags(Vec<String>),
    Notes(Vec<Note>),
}

impl PreImage {
    /// Whether `other` is an image of the same record.
    pub(crate) fn is_same_record(&self, other: &PreImage) -> bool {
        discriminant(self) == discriminant(other) && self.id() == other.id()
    }

    fn id(&self) -> &str {
        match self {
            PreImage::Actor(id, _) | PreImage::Place(id, _) | PreImage::Story(id, _) => id,
            PreImage::WorldMeters(_) | PreImage::WorldFlags(_) | PreImage::Notes(_) => "",
        }
    }

    /// Note every key whose value differs between this image and the record as it is now.
    pub(crate) fn diff_to_now(&self, canon: &Canon, state: &State, out: &mut Diff) {
        match self {
            PreImage::Actor(id, before) => {
                diff_now(&ActorId(id), before.as_deref(), canon, state, out)
            }
            PreImage::Place(id, before) => {
                diff_now(&PlaceId(id), before.as_ref(), canon, state, out)
            }
            PreImage::Story(id, before) => {
                diff_now(&StoryId(id), before.as_ref(), canon, state, out)
            }
            PreImage::WorldMeters(before) => {
                diff_now(&WorldMetersId, Some(before), canon, state, out)
            }
            PreImage::WorldFlags(before) => {
                diff_now(&WorldFlagsId, Some(before), canon, state, out)
            }
            PreImage::Notes(before) => diff_now(&NotesId, Some(before), canon, state, out),
        }
    }
}

/// Compare `before` with `id`'s record now.
fn diff_now<I: RecordId>(
    id: &I,
    before: Option<&I::Record>,
    canon: &Canon,
    state: &State,
    out: &mut Diff,
) {
    if let Some(after) = id.get(state) {
        id.diff(canon, before, after, out);
    }
}

impl State {
    /// Put a record back as its pre-image has it.
    pub(crate) fn restore(&mut self, image: PreImage) {
        fn put<T>(map: &mut BTreeMap<String, T>, id: String, record: Option<T>) {
            match record {
                Some(record) => map.insert(id, record),
                None => map.remove(&id),
            };
        }
        match image {
            PreImage::Actor(id, record) => put(&mut self.actors, id, record.map(|boxed| *boxed)),
            PreImage::Place(id, record) => put(&mut self.places, id, record),
            PreImage::Story(id, record) => put(&mut self.stories, id, record),
            PreImage::WorldMeters(meters) => self.world_meters = meters,
            PreImage::WorldFlags(flags) => self.flags = flags,
            PreImage::Notes(notes) => self.notes = notes,
        }
    }
}

impl RecordId for ActorId<'_> {
    type Record = ActorLive;

    fn get<'s>(&self, state: &'s State) -> Option<&'s ActorLive> {
        state.actors.get(self.0)
    }

    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut ActorLive, Reason> {
        state.actors.get_mut(self.0).ok_or(Reason::UnknownId)
    }

    fn pre_image(&self, before: Option<ActorLive>) -> PreImage {
        PreImage::Actor(self.0.to_string(), before.map(Box::new))
    }

    /// `kind` never changes, and `visited` is the map's own record, with no key of its own.
    /// An actor is never created in a round, so `before` is always there.
    fn diff(&self, canon: &Canon, before: Option<&ActorLive>, after: &ActorLive, out: &mut Diff) {
        let Some(before) = before else {
            return;
        };
        let id = self.0;
        out.stored(StateKey::ActorPlace, id, &before.place, &after.place);
        diff_flags(
            &before.flags,
            &after.flags,
            |flag| StateKey::ActorFlag(id.to_string(), flag),
            out,
        );
        diff_meters(
            canon,
            &Holder::Actor(id.to_string()),
            before.into(),
            after.into(),
            |key| StateKey::ActorMeter(id.to_string(), key),
            out,
        );
        for key in union(before.modifiers.keys(), after.modifiers.keys()) {
            let (held, now) = (before.modifiers.get(key), after.modifiers.get(key));
            out.stored(
                |key| StateKey::ActorModifier(id.to_string(), key),
                key,
                held,
                now,
            );
        }
        // Busy stores the tick and the action under way; its value is the tick.
        out.note(
            StateKey::ActorBusy,
            id,
            (before.act.busy_until, &before.act.pending)
                != (after.act.busy_until, &after.act.pending),
            before.act.busy_until.into(),
            after.act.busy_until.into(),
        );
        out.stored(
            StateKey::ActorKnownFact,
            id,
            &before.known_facts,
            &after.known_facts,
        );
        out.stored(
            StateKey::LastActRound,
            id,
            before.act.last_act_round,
            after.act.last_act_round,
        );
        out.stored(
            StateKey::IdleActsThisWindow,
            id,
            before.act.idle_acts_this_window,
            after.act.idle_acts_this_window,
        );
        out.stored(
            StateKey::LiveActsThisWindow,
            id,
            before.act.live_acts_this_window,
            after.act.live_acts_this_window,
        );
        diff_counts(
            &before.inventory,
            &after.inventory,
            |item| StateKey::ActorItem(id.to_string(), item),
            out,
        );
        out.stored(
            StateKey::ActorWielded,
            id,
            before.wielded.as_ref(),
            after.wielded.as_ref(),
        );
    }
}

impl RecordId for PlaceId<'_> {
    type Record = PlaceLive;

    fn get<'s>(&self, state: &'s State) -> Option<&'s PlaceLive> {
        state.places.get(self.0)
    }

    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut PlaceLive, Reason> {
        Ok(state.places.entry(self.0.to_string()).or_default())
    }

    fn pre_image(&self, before: Option<PlaceLive>) -> PreImage {
        PreImage::Place(self.0.to_string(), before)
    }

    /// A place that was not there compares as an empty one.
    fn diff(&self, canon: &Canon, before: Option<&PlaceLive>, after: &PlaceLive, out: &mut Diff) {
        let empty = PlaceLive::default();
        let before = before.unwrap_or(&empty);
        let id = self.0;
        diff_counts(
            &before.items,
            &after.items,
            |item| StateKey::PlaceItem(id.to_string(), item),
            out,
        );
        diff_meters(
            canon,
            &Holder::Place(id.to_string()),
            before.into(),
            after.into(),
            |key| StateKey::PlaceMeter(id.to_string(), key),
            out,
        );
        diff_flags(
            &before.flags,
            &after.flags,
            |flag| StateKey::PlaceFlag(id.to_string(), flag),
            out,
        );
        diff_flags(
            &before.open,
            &after.open,
            |exit| StateKey::ExitOpen(id.to_string(), exit),
            out,
        );
    }
}

impl RecordId for StoryId<'_> {
    type Record = StoryLive;

    fn get<'s>(&self, state: &'s State) -> Option<&'s StoryLive> {
        state.stories.get(self.0)
    }

    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut StoryLive, Reason> {
        Ok(state.stories.entry(self.0.to_string()).or_default())
    }

    fn pre_image(&self, before: Option<StoryLive>) -> PreImage {
        PreImage::Story(self.0.to_string(), before)
    }

    /// A story that was not there compares as an empty one.
    fn diff(&self, canon: &Canon, before: Option<&StoryLive>, after: &StoryLive, out: &mut Diff) {
        let empty = StoryLive::default();
        let before = before.unwrap_or(&empty);
        let id = self.0;
        diff_meters(
            canon,
            &Holder::Story(id.to_string()),
            before.into(),
            after.into(),
            |key| StateKey::StoryMeter(id.to_string(), key),
            out,
        );
        diff_flags(
            &before.flags,
            &after.flags,
            |flag| StateKey::StoryFlag(id.to_string(), flag),
            out,
        );
    }
}

impl RecordId for WorldMetersId {
    type Record = BTreeMap<String, MeterLive>;

    fn get<'s>(&self, state: &'s State) -> Option<&'s Self::Record> {
        Some(&state.world_meters)
    }

    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut Self::Record, Reason> {
        Ok(&mut state.world_meters)
    }

    fn pre_image(&self, before: Option<Self::Record>) -> PreImage {
        PreImage::WorldMeters(before.unwrap_or_default())
    }

    fn diff(
        &self,
        canon: &Canon,
        before: Option<&Self::Record>,
        after: &Self::Record,
        out: &mut Diff,
    ) {
        let empty = BTreeMap::new();
        diff_meters(
            canon,
            &Holder::World,
            Meters::new(before.unwrap_or(&empty), None),
            Meters::new(after, None),
            StateKey::WorldMeter,
            out,
        );
    }
}

impl RecordId for WorldFlagsId {
    type Record = Vec<String>;

    fn get<'s>(&self, state: &'s State) -> Option<&'s Vec<String>> {
        Some(&state.flags)
    }

    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut Vec<String>, Reason> {
        Ok(&mut state.flags)
    }

    fn pre_image(&self, before: Option<Vec<String>>) -> PreImage {
        PreImage::WorldFlags(before.unwrap_or_default())
    }

    fn diff(&self, _: &Canon, before: Option<&Vec<String>>, after: &Vec<String>, out: &mut Diff) {
        diff_flags(
            before.map_or(&[], Vec::as_slice),
            after,
            StateKey::WorldFlag,
            out,
        );
    }
}

impl RecordId for NotesId {
    type Record = Vec<Note>;

    fn get<'s>(&self, state: &'s State) -> Option<&'s Vec<Note>> {
        Some(&state.notes)
    }

    fn get_mut<'s>(&self, state: &'s mut State) -> Result<&'s mut Vec<Note>, Reason> {
        Ok(&mut state.notes)
    }

    fn pre_image(&self, before: Option<Vec<Note>>) -> PreImage {
        PreImage::Notes(before.unwrap_or_default())
    }

    fn diff(&self, _: &Canon, before: Option<&Vec<Note>>, after: &Vec<Note>, out: &mut Diff) {
        fn ids(notes: &[Note]) -> impl Iterator<Item = &String> {
            notes.iter().map(|note| &note.id)
        }
        let before = before.map_or(&[][..], Vec::as_slice);
        for id in union(ids(before), ids(after)) {
            out.stored(
                StateKey::Note,
                id,
                find_note(before, id),
                find_note(after, id),
            );
        }
    }
}

/// The ids on either side, each once, in order.
fn union<'a, T: AsRef<str> + 'a>(
    before: impl IntoIterator<Item = &'a T>,
    after: impl IntoIterator<Item = &'a T>,
) -> BTreeSet<&'a str> {
    before.into_iter().chain(after).map(AsRef::as_ref).collect()
}

/// A flag list: each name on either side is one key, set or clear.
fn diff_flags(
    before: &[String],
    after: &[String],
    key_of: impl Fn(String) -> StateKey,
    out: &mut Diff,
) {
    for flag in union(before, after) {
        out.stored(&key_of, flag, is_set(before, flag), is_set(after, flag));
    }
}

/// An item count list: a missing entry counts 0.
fn diff_counts(
    before: &BTreeMap<String, u32>,
    after: &BTreeMap<String, u32>,
    key_of: impl Fn(String) -> StateKey,
    out: &mut Diff,
) {
    for item in union(before.keys(), after.keys()) {
        out.stored(&key_of, item, count_of(before, item), count_of(after, item));
    }
}

/// The meters a holder carries. A meter's stored fields are its base and offset; its value is
/// `Meters::value`, which an actor's modifiers can move while the fields hold.
fn diff_meters(
    canon: &Canon,
    holder: &Holder,
    before: Meters,
    after: Meters,
    key_of: impl Fn(String) -> StateKey,
    out: &mut Diff,
) {
    for key in union(before.live.keys(), after.live.keys()) {
        out.note(
            &key_of,
            key,
            before.live.get(key) != after.live.get(key),
            meter_value(canon, holder, before, key),
            meter_value(canon, holder, after, key),
        );
    }
}

/// Meter `key`'s value as a watched value: `Empty` when the pack or the holder lacks it.
fn meter_value(canon: &Canon, holder: &Holder, meters: Meters, key: &str) -> StateValue {
    canon
        .meter_def(holder, key)
        .and_then(|def| meters.value(def, key).ok())
        .into()
}
