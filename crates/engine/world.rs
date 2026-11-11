//! An open pack, and the check that reads one without writing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spookery_protocol::{CommandView, PlaceView};

use crate::canon::Budget;
use crate::canon::{check_save, load_canon, pack_rev, Canon};
use crate::diagnostic::{error, first_reason, locate_nth, schema_error, Code, Diagnostic};
use crate::intent::{Intent, IntentArgs, Source, Target, Viewer};
use crate::rules::context::Context;
use crate::rules::effect::{payload_json, Effect};
use crate::rules::gate;
use crate::rules::rule::{Rule, TargetKind, When};
use crate::state::id::{ActorId, EntityId};
use crate::state::{ActorKind, Presence, State};
use crate::store::journal;
use crate::store::records::Record;
use crate::store::save;
use crate::store::tape::{append_record, read_scrollback, TapeRecord};
use crate::Reason;
use spookery_protocol::{QueuedMove, Speaker, PERCEIVE_BODY_CHARS};

/// One say, in characters. Four kibibytes still fits in an 8 KiB client message.
const MAX_SAY_CHARS: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address<'a> {
    Table,
    Place(&'a str),
    Actor(&'a str),
}

pub struct World<L = crate::PackGuard> {
    pub(crate) root: PathBuf,
    pub(crate) canon: Arc<Canon>,
    pub(crate) state: State,
    pub(crate) queue: Vec<Intent>,
    warning: Option<&'static str>,
    pub(crate) lock: L,
}

impl World {
    /// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
    /// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
    pub fn resume(root: &Path, lock: crate::PackGuard) -> Result<Self, Reason> {
        let World {
            root,
            canon,
            state,
            queue,
            warning,
            lock: (),
        } = crate::round::replay::open_at_tip(root, &lock)?;
        Ok(World {
            root,
            canon,
            state,
            queue,
            warning,
            lock,
        })
    }

    /// Play the queue with `presence` seated.
    pub fn play(
        &mut self,
        presence: &Presence,
        now_wall: &str,
    ) -> Result<crate::RoundOutput, Reason> {
        crate::round::play_round(self, presence, now_wall, false)
    }

    /// Commit a queued round, then fast-forward while every seated PC is busy.
    /// An empty queue only fast-forwards.
    pub fn commit_then_fast_forward(
        &mut self,
        presence: &Presence,
        wall: &str,
    ) -> Result<Vec<crate::RoundOutput>, Reason> {
        let mut outputs = Vec::new();
        let committed = !self.queue.is_empty();
        if committed {
            outputs.push(self.play(presence, wall)?);
        }
        let already = u32::from(committed);
        outputs.extend(crate::round::fast_forward(self, presence, wall, already)?);
        Ok(outputs)
    }

    /// Play the rounds an absence earns. Only an empty table catches up.
    pub fn catch_up(&mut self, now_wall: &str, downtime_hours: u64) -> Result<(), Reason> {
        crate::round::play_catch_up(self, now_wall, downtime_hours)
    }

    /// Append a player's say. `presence` is who can hear it.
    pub fn say_as_player(
        &self,
        presence: &Presence,
        actor_id: &ActorId,
        heard: Address<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        if self.state.actors().kind.get(actor_id) != Some(&ActorKind::Pc) {
            return Err(Reason::Schema);
        }
        let from = Speaker::Player(actor_id.to_string());
        self.append_speech(presence, from, Some(actor_id), heard, text)
    }

    pub fn say_as_gm(
        &self,
        presence: &Presence,
        heard: Address<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        self.append_speech(presence, Speaker::Gm, None, heard, text)
    }

    fn append_speech(
        &self,
        presence: &Presence,
        from: Speaker,
        speaker: Option<&ActorId>,
        heard: Address<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        let text = check_say_text(text)?.to_string();
        if let Address::Actor(actor) = heard {
            if !self.state.actors().kind.contains_key(actor) {
                return Err(Reason::Schema);
            }
        }
        let (address, place, actor) = match heard {
            Address::Table => ("table", None, None),
            Address::Place(place) => ("place", Some(place.to_string()), None),
            Address::Actor(actor) => ("actor", None, Some(actor.to_string())),
        };
        let recipients = speech_recipients(&self.state, presence, speaker, heard);
        let record = TapeRecord {
            seq: 0,
            from,
            address: address.to_string(),
            place,
            actor,
            recipients,
            text,
            round_index: self.state.round_index,
            world_time: self.state.world_time,
        };
        append_record(&self.root, record)
    }
}

impl<L> World<L> {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn canon(&self) -> &Canon {
        &self.canon
    }

    pub fn pack_summary(&self) -> spookery_protocol::PackSummary {
        self.canon.to_pack_summary()
    }

    /// Every verb the pack's rules name: the commands a player seat may send as moves.
    pub fn verbs(&self) -> Vec<String> {
        self.canon.verbs()
    }

    /// Every PC in the cast.
    pub fn pcs(&self) -> impl Iterator<Item = &ActorId> {
        self.state
            .actors()
            .kind
            .iter()
            .filter(|(_, kind)| **kind == ActorKind::Pc)
            .map(|(id, _)| id)
    }

    /// The PC `typed` names, by id or display name, ignoring case and outer spaces. `None`
    /// when no PC has that name, and `AMBIGUOUS_NAME` when two do.
    pub fn find_pc(&self, typed: &str) -> Result<Option<ActorId>, Reason> {
        let pcs = self
            .pcs()
            .map(|id| (id.as_str(), self.canon.display_name(id.as_str())));
        let pcs: Vec<gate::NamedId> = pcs.map(|(id, name)| gate::NamedId::new(id, name)).collect();
        Ok(gate::find_named(&pcs, typed)?.map(ActorId::from))
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Replace the in-memory state after a replay. This does not write `save.yaml`.
    pub(crate) fn set_state(&mut self, state: State) {
        self.state = state;
    }

    /// Gate a player's line with `presence` seated, then queue it. The line is an intent
    /// from the start, so each refusal names its id. A typed name stands for the one id in
    /// sight that has it, and a name two of them share is `AMBIGUOUS_NAME`. Only a save at
    /// the last round index has no next round to name, and refuses `SCHEMA` with an empty id.
    pub fn queue_player_move(
        &mut self,
        presence: &Presence,
        actor_id: &ActorId,
        verb: &str,
        target: Option<&str>,
    ) -> Result<QueuedMove, gate::Refusal> {
        let typed = target.filter(|token| !token.is_empty());
        let named = match typed {
            Some(typed) if verb != gate::CANCEL => {
                let viewer = Viewer::Player(actor_id.clone());
                let nameable = gate::list_nameable(&self.canon, &self.state, presence, &viewer);
                gate::find_named(&nameable, typed).map(|id| id.map(String::from))
            }
            _ => Ok(None),
        };
        let resolved = named.clone().unwrap_or_default();
        let token = resolved.as_deref().or(typed);
        let Ok(mut intent) = Intent::player(&self.state, actor_id, verb, token) else {
            return Err(gate::Refusal {
                id: String::new(),
                code: Reason::Schema,
                detail: None,
            });
        };
        if let Err(code) = named {
            let detail = typed.map(|typed| typed.trim().to_string());
            return Err(gate::Refusal {
                detail,
                ..gate::Refusal::new(&intent, code)
            });
        }
        if gate::is_cancel(&intent) {
            if token.is_some() || self.queue.iter().any(|queued| queued.id == intent.id) {
                return Err(gate::Refusal::new(&intent, Reason::Schema));
            }
            gate::admit_cancel(&Context::new(&self.canon, &self.state, presence, &intent))?;
        } else {
            let shaped = self.shape_line(&intent, token);
            intent.target = shaped.map_err(|code| gate::Refusal::new(&intent, code))?;
            let context = Context::new(&self.canon, &self.state, presence, &intent);
            let rule = gate::rule_of(&context)?;
            gate::admit(&context, rule)?;
        }
        let queued = QueuedMove {
            id: intent.id.clone(),
            actor_id: actor_id.to_string(),
            move_name: verb.to_string(),
        };
        self.queue_intent(intent);
        Ok(queued)
    }

    /// The target a PC's line names for the rule it picks. A line already queued is `SCHEMA`.
    fn shape_line(&self, line: &Intent, token: Option<&str>) -> Result<Target, Reason> {
        let Some(viewer) = &line.actor else {
            return Err(Reason::Schema);
        };
        if self.state.actors().kind.get(viewer.actor_id()) != Some(&ActorKind::Pc) {
            return Err(Reason::Schema);
        }
        let (actor, verb) = (viewer, &line.move_name);
        let rule = gate::pick(&self.canon, &self.state, actor, line.source, verb, token)?;
        let target = target_for(rule, token)?;
        if self.queue.iter().any(|queued| queued.id == line.id) {
            return Err(Reason::Schema);
        }
        Ok(target)
    }

    /// Pick the GM's intent's rule, admit it with `presence` seated, then queue it. A refusal
    /// is its code.
    pub fn queue_gm_intent(
        &mut self,
        presence: &Presence,
        spec: spookery_protocol::IntentArgs,
        mint_id: impl FnOnce() -> Result<String, Reason>,
    ) -> Result<QueuedMove, Reason> {
        let actor_id: ActorId = spec
            .actor_id
            .filter(|id| !id.is_empty() && !id.contains(':'))
            .ok_or(Reason::Schema)?
            .into();
        let move_name = spec
            .move_name
            .filter(|m| !m.is_empty())
            .ok_or(Reason::Schema)?;
        let viewer = self.state.viewer_of(&actor_id).ok_or(Reason::UnknownId)?;
        let token = spec.target.as_deref().filter(|t| !t.is_empty());
        let (canon, state) = (&self.canon, &self.state);
        let rule = gate::pick(canon, state, &viewer, Source::Gm, &move_name, token)?;
        let target = target_for(rule, token)?;
        let id = match spec.id {
            Some(id) if !id.is_empty() => id,
            Some(_) => return Err(Reason::Schema),
            None => mint_id()?,
        };
        if self.queue.iter().any(|intent| intent.id == id) {
            return Err(Reason::Schema);
        }
        let round_index = match spec.round_index {
            Some(index) => index,
            None => self
                .state
                .round_index
                .checked_add(1)
                .ok_or(Reason::Schema)?,
        };
        let args = IntentArgs {
            message_to: spec
                .message_to
                .map(|ids| ids.into_iter().map(ActorId::from).collect()),
        };
        let intent = Intent {
            id: id.clone(),
            round_index,
            actor: Some(viewer),
            source: Source::Gm,
            move_name: move_name.clone(),
            target,
            args,
            rationale_code: spec.rationale_code,
        };
        let context = Context::new(canon, state, presence, &intent);
        gate::admit(&context, rule).map_err(|refusal| refusal.code)?;
        self.queue_intent(intent);
        Ok(QueuedMove {
            id,
            actor_id: actor_id.into(),
            move_name,
        })
    }

    pub fn player_scrollback(&self, actor_id: &ActorId) -> Result<Vec<TapeRecord>, Reason> {
        read_scrollback(&self.root, actor_id.as_str())
    }

    pub fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub(crate) fn queue_intent(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    /// A committed round whose checkpoint file was not finished. The host prints it.
    pub fn take_warning(&mut self) -> Option<&'static str> {
        self.warning.take()
    }

    pub(crate) fn set_warning(&mut self, text: &'static str) {
        self.warning = Some(text);
    }

    pub fn perceive(
        &self,
        presence: &Presence,
        as_arg: &str,
    ) -> Result<crate::rules::perceive::View, Reason> {
        crate::rules::perceive::perceive_budget(self, presence, as_arg, PERCEIVE_BODY_CHARS)
    }

    /// The `help` commands for `actor_id`: what they can do now, and what they can name.
    pub fn help(
        &self,
        presence: &Presence,
        actor_id: &ActorId,
    ) -> Result<Vec<CommandView>, Reason> {
        crate::rules::lines::help(&self.canon, &self.state, presence, actor_id)
    }

    /// The `map` for `actor_id`: each place they visited, with the exits they saw there last.
    pub fn map(&self, actor_id: &ActorId) -> Result<Vec<PlaceView>, Reason> {
        crate::rules::lines::map(&self.canon, &self.state, actor_id)
    }

    pub fn perceive_trimmed(
        &self,
        presence: &Presence,
        as_arg: &str,
        max_body_chars: usize,
    ) -> Result<crate::rules::perceive::View, Reason> {
        crate::rules::perceive::perceive_budget(self, presence, as_arg, max_body_chars)
    }

    fn read_digest(&self) -> Result<String, Reason> {
        let entries = journal::read_verified(&self.root).map_err(|fault| fault.reason)?;
        let mut effects = Vec::new();
        for entry in &entries {
            let Record::Round(record) = entry else {
                continue;
            };
            for effect in &record.effects {
                effects.push((record.round_index, effect));
            }
        }
        Ok(self.format_digest(&effects))
    }

    /// One row per meter that one actor, place or story, or the world, carries:
    /// `meter <entity> <id> <value> <max>`, the entity spelled as a rule's `who` names it.
    /// A working copy of the save with a round open on it under the pack's budget. A stage that
    /// refuses, or a commit that fails, drops the copy and the save stays as it was.
    pub(crate) fn begin_round(&self) -> State {
        let mut state = self.state.clone();
        state.open_round(&self.canon.budget);
        state
    }

    fn carried_meter_rows(&self) -> Vec<String> {
        let carried = self
            .canon
            .meters
            .iter()
            .filter_map(|(id, def)| Some((def.carrier.clone()?, id)));
        let world_meters = self
            .canon
            .world_meters
            .keys()
            .map(|id| (EntityId::World, id));
        carried
            .chain(world_meters)
            .filter_map(|(entity, id)| {
                let value = self
                    .state
                    .entity(&entity)
                    .meter_value(&self.canon, id)
                    .ok()?;
                let max = self.canon.meter_def(entity.as_ref(), id)?.max;
                Some(format!("meter {entity} {id} {value} {max}"))
            })
            .collect()
    }

    /// Effect rows for `outputs`, which end at this world's `round_index`, then the meter rows.
    pub fn digest(&self, outputs: &[crate::RoundOutput]) -> String {
        let start = self.state.round_index + 1 - outputs.len() as u64;
        let mut effects = Vec::new();
        for (offset, output) in outputs.iter().enumerate() {
            let round_index = start + offset as u64;
            for effect in &output.effects {
                effects.push((round_index, effect));
            }
        }
        self.format_digest(&effects)
    }

    fn format_digest(&self, effects: &[(u64, &Effect)]) -> String {
        let mut rows = Vec::new();
        for (round_index, effect) in effects {
            rows.push(format!(
                "effect {} {round_index} {}",
                effect.kind.as_str(),
                payload_json(effect)
            ));
        }
        rows.extend(self.carried_meter_rows());
        if rows.is_empty() {
            String::new()
        } else {
            rows.join("\n") + "\n"
        }
    }
}

impl World<()> {
    /// Open the pack at the journal tip and write nothing, so the pack lock is not needed.
    /// A read guard waits out a write in progress and drops once the World is built.
    /// A crash the writer has not recovered is read as it stands.
    ///
    /// A peeked World holds no lock, so it cannot play a round:
    ///
    /// ```compile_fail
    /// # let root = std::path::Path::new("pack");
    /// let presence = spookery_engine::Presence::default();
    /// spookery_engine::World::peek(root).unwrap().play(&presence, "");
    /// ```
    pub fn peek(root: &Path) -> Result<World<()>, Reason> {
        let _read = crate::store::lock::lock_read(root)?;
        crate::round::replay::peek_at_tip(root)
    }

    /// The host text at the journal tip: effect rows from the journal, then the meter rows.
    /// One read guard covers the World and the journal read, so both come from one state.
    pub fn peek_digest(root: &Path) -> Result<String, Reason> {
        let _read = crate::store::lock::lock_read(root)?;
        crate::round::replay::peek_at_tip(root)?.read_digest()
    }

    /// Open the pack. A save that later journal records continue is a checkpoint,
    /// and resume checks the tip instead.
    pub fn open(root: &Path) -> Result<Self, Reason> {
        if !root.join("pack.yaml").is_file() {
            return Err(Reason::NotInSpec);
        }
        let now_bytes = journal::read_save(&journal::save_path(root))?;
        let canon = Arc::new(load_canon(root).map_err(|diags| first_reason(&diags))?);
        let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
        let state = save::parse(now_text).map_err(|diags| first_reason(&diags))?;
        let live = journal::read_live_records(root).unwrap_or_default();
        let mut problems = check_save_text(&state, &now_bytes);
        if let Some(diag) = check_pack_rev(root, tip_pack_rev(&live, &state), &now_bytes) {
            problems.insert(0, diag);
        }
        let continues = journal_continues(&live, &state);
        if continues {
            problems.retain(|diag| diag.code != Code::Error(Reason::HandEdit));
        }
        if !problems.is_empty() {
            return Err(first_reason(&problems));
        }
        if !continues {
            check_save(&canon, &state).map_err(|diags| first_reason(&diags))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            canon,
            state,
            queue: Vec::new(),
            warning: None,
            lock: (),
        })
    }

    /// The pack's canon with a save from memory, for replay. Nothing is checked:
    /// each journal record's hash proves the state it reaches.
    pub(crate) fn from_parts(pack: &Path, canon: Arc<Canon>, state: State) -> Self {
        Self {
            root: pack.to_path_buf(),
            canon,
            state,
            queue: Vec::new(),
            warning: None,
            lock: (),
        }
    }
}

/// Seated players who hear this say. The speaker is included.
fn speech_recipients(
    state: &State,
    presence: &Presence,
    speaker: Option<&ActorId>,
    heard: Address<'_>,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    let seated = &presence.actors_present;
    match heard {
        Address::Table => {
            ids.extend(seated.iter().map(ActorId::to_string));
        }
        Address::Place(place) => {
            let here = state.actors_at(place);
            ids.extend(
                seated
                    .iter()
                    .filter(|id| here.contains(id))
                    .map(ActorId::to_string),
            );
        }
        Address::Actor(actor) => {
            if seated.iter().any(|id| *id == *actor) {
                ids.insert(actor.to_string());
            }
        }
    }
    if let Some(speaker) = speaker {
        ids.insert(speaker.to_string());
    }
    ids.into_iter().collect()
}

/// The target a typed token is for `rule`: none for a habit or a verb that takes none, else
/// the token as the kind the verb takes. A token where none fits, or none where one is
/// needed, is `SCHEMA`.
fn target_for(rule: &Rule, token: Option<&str>) -> Result<Target, Reason> {
    let kind = match &rule.when {
        When::Verb { target, .. } => *target,
        When::Habit | When::Change(_) => TargetKind::None,
    };
    match (kind, token) {
        (TargetKind::None, None) => Ok(Target::Empty),
        (TargetKind::None, Some(_)) | (_, None) => Err(Reason::Schema),
        (kind, Some(id)) => Ok(Target::new(kind, id)),
    }
}

fn check_say_text(text: &str) -> Result<&str, Reason> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > MAX_SAY_CHARS {
        return Err(Reason::Schema);
    }
    Ok(text)
}

/// Hash of the newest migrate in `live` past the checkpoint, or the save's own hash.
fn tip_pack_rev<'a>(live: &'a [Record], state: &'a State) -> &'a str {
    live.iter()
        .rev()
        .filter(|entry| entry.state_rev_after() > state.state_rev)
        .find_map(|entry| match entry {
            Record::Migrate(record) => Some(record.pack_rev_after.as_str()),
            _ => None,
        })
        .unwrap_or(&state.pack_rev)
}

/// `Some` when the canon files are not the journal tip's hash.
fn check_pack_rev(root: &Path, tip: &str, bytes: &[u8]) -> Option<Diagnostic> {
    match pack_rev(root) {
        Ok(rev) if tip != rev => {
            let mut diag = error(
                "state/save.yaml",
                Reason::PackRevMismatch,
                "pack_rev does not match the canon hash",
                "run spook migrate to store the current hash",
            )
            .at_path("pack_rev");
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some((line, column)) = locate_nth(text, "pack_rev:", 0) {
                    diag = diag.at_line(line, Some(column));
                }
            }
            Some(diag)
        }
        Ok(_) => None,
        Err(_) => Some(
            error(
                "state/save.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .at_path("pack_rev"),
        ),
    }
}

/// The on-disk save is a checkpoint. Later live records still have to be replayed.
fn journal_continues(live: &[Record], state: &State) -> bool {
    live.iter().any(|entry| match entry {
        Record::Checkpoint(_) => false,
        _ => entry.state_rev_after() > state.state_rev,
    })
}

/// Read the pack and report every loader and lint problem. Writes nothing.
pub fn check_pack(root: &Path) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let canon = match load_canon(root) {
        Ok(canon) => Some(canon),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    let now_path = journal::save_path(root);
    let loaded = match journal::read_save(&now_path) {
        Ok(bytes) => match std::str::from_utf8(&bytes) {
            Ok(text) => match save::parse(text) {
                Ok(state) => Some((bytes, state)),
                Err(more) => {
                    diags.extend(more);
                    None
                }
            },
            Err(_) => {
                diags.push(
                    schema_error(
                        "state/save.yaml",
                        "this file is not UTF-8 text",
                        "save it as UTF-8",
                    )
                    .at_path("schema"),
                );
                None
            }
        },
        Err(Reason::NotInSpec) => None,
        Err(reason) => {
            diags.push(
                error(
                    "state/save.yaml",
                    reason,
                    "state/save.yaml could not be read",
                    "check that the file is readable",
                )
                .at_path("schema"),
            );
            None
        }
    };
    if let Some((bytes, state)) = &loaded {
        let live = journal::read_live_records(root).unwrap_or_default();
        let mut problems = check_save_text(state, bytes);
        // A migrate record past the checkpoint has already accepted the save's edit.
        let migrating = live.iter().any(|entry| {
            matches!(entry, Record::Migrate(_)) && entry.state_rev_after() > state.state_rev
        });
        if migrating {
            problems.retain(|diag| diag.code != Code::Error(Reason::HandEdit));
        }
        if let Some(diag) = check_pack_rev(root, tip_pack_rev(&live, state), bytes) {
            problems.insert(0, diag);
        }
        diags.extend(problems);
        // A checkpoint save is not the tip. Resume checks the tip when it opens.
        if let Some(canon) = canon.as_ref().filter(|_| !journal_continues(&live, state)) {
            if let Err(more) = check_save(canon, state) {
                diags.extend(more);
            }
        }
    }
    if let Some(canon) = &canon {
        diags.extend(crate::canon::check_canon(canon));
    }
    diags
}

fn check_save_text(state: &State, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if state.actors().kind.contains_key("world") {
        diags.push(
            schema_error(
                "state/save.yaml",
                "actor id world is reserved",
                "rename that actor",
            )
            .at_path("actors.world"),
        );
    }
    match save::emit(state) {
        Ok(emitted) if emitted.as_bytes() != bytes => {
            let line = std::str::from_utf8(bytes)
                .ok()
                .and_then(|live| first_diff_line(live, &emitted).map(|(line_no, _)| line_no));
            let mut diag = error(
                "state/save.yaml",
                Reason::HandEdit,
                "this save does not match the canonical text",
                "run spook migrate to keep the edit, or restore the file",
            );
            diag = match line {
                Some(line) => diag.at_line(line, None),
                None => diag.at_path("save"),
            };
            diags.push(diag);
        }
        Ok(_) => {}
        Err(_) => diags.push(
            error(
                "state/save.yaml",
                Reason::Schema,
                "the save could not be written back as YAML",
                "fix the values so they match the save format",
            )
            .at_path("save"),
        ),
    }
    diags
}

/// First 1-based line number and line text from `live` that differs from `expected`.
pub(crate) fn first_diff_line(live: &str, expected: &str) -> Option<(u32, String)> {
    for (index, (left, right)) in live.lines().zip(expected.lines()).enumerate() {
        if left != right {
            return Some(((index + 1) as u32, left.to_string()));
        }
    }
    let live_count = live.lines().count();
    let expected_count = expected.lines().count();
    if live_count != expected_count {
        let number = live_count.min(expected_count) + 1;
        let line = live
            .lines()
            .nth(number - 1)
            .or_else(|| expected.lines().nth(number - 1))
            .unwrap_or("")
            .to_string();
        return Some((number as u32, line));
    }
    if live.as_bytes() != expected.as_bytes() {
        return Some((1, live.lines().next().unwrap_or("").to_string()));
    }
    None
}
