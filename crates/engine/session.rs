//! An open pack, and the check that reads one without writing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spookery_protocol::{CommandView, PlaceView};

use crate::canon::Budget;
use crate::canon::{load_canon, pack_rev, Canon};
use crate::diagnostic::{error, first_reason, locate_nth, schema_error, Code, Diagnostic};
use crate::ecs::id::{ActorId, PlaceId};
use crate::ecs::{self, component, Presence, Save};
use crate::entity;
use crate::intent::{Intent, IntentArgs, Source, Target, Viewer};
use crate::rules::context::Context;
use crate::rules::effect::{payload_json, Effect};
use crate::rules::gate;
use crate::rules::rule::{Rule, TargetKind, When};
use crate::store::journal;
use crate::store::records::Record;
use crate::store::save;
use crate::store::tape::{append_record, read_scrollback, TapeRecord};
use crate::Reason;
use spookery_protocol::{Audience, QueuedMove, Refusal, Speaker, PERCEIVE_BODY_CHARS};

/// One say, in characters. Four kibibytes still fits in an 8 KiB client message.
const MAX_SAY_CHARS: usize = 4 * 1024;

/// An open pack: its root, the ECS world its save reads into, the queued intents and the pack
/// lock `Lock`, which is `()` for a session that only reads.
pub struct Session<Lock = crate::PackGuard> {
    pub(crate) root: PathBuf,
    pub(crate) world: ecs::World,
    pub(crate) queue: Vec<Intent>,
    pub(crate) lock: Lock,
}

impl Session {
    /// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
    /// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
    pub fn resume(root: &Path, lock: crate::PackGuard) -> Result<Self, Reason> {
        let Session {
            root,
            world,
            queue,
            lock: (),
        } = crate::round::replay::open_at_tip(root, &lock)?;
        tracing::info!(
            pack_rev = world.pack_rev.as_str(),
            state_rev = world.state_rev,
            round_index = world.round_index,
            world_time = world.world_time,
            "pack opened"
        );
        Ok(Session {
            root,
            world,
            queue,
            lock,
        })
    }

    /// Play the queue with `presence` seated.
    pub fn play(
        &mut self,
        presence: &Presence,
        now_wall: &str,
    ) -> Result<crate::round::Output, Reason> {
        crate::round::play_round(self, presence, now_wall, false)
    }

    /// Commit a queued round, then fast-forward while every seated PC is busy.
    /// An empty queue only fast-forwards.
    pub fn commit_then_fast_forward(
        &mut self,
        presence: &Presence,
        wall: &str,
    ) -> Result<Vec<crate::round::Output>, Reason> {
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

    /// Append a player's speech. `presence` is who can hear it. Speech to an actor reads
    /// `opening` and `text` as one line, and its listener is the longest run of leading words
    /// that names an actor in sight, by id or name, as a move's target is; the rest is
    /// the text. A name two of them share is `AMBIGUOUS_NAME`, and a line that names nobody is
    /// said at the table, its text the whole line.
    pub fn say_as_player(
        &self,
        presence: &Presence,
        actor_id: &ActorId,
        audience: &Audience,
        text: &str,
    ) -> Result<TapeRecord, Refusal> {
        let actors = self.world.actors();
        if !actors.get::<component::ControlType>(actor_id).is_pc() {
            return Err(Reason::Schema.into());
        }
        let from = Speaker::Player(actor_id.to_string());
        let Audience::Actor(opening) = audience else {
            return Ok(self.append_speech(presence, from, Some(actor_id), Some(audience), text)?);
        };
        let viewer = Viewer::Player(actor_id.clone());
        let mut ids = gate::list_ids(self.canon(), &self.world, presence, &viewer);
        let actors = self.world.actors();
        ids.retain(|named_id| actors.contains(&ActorId::from(named_id.id.as_str())));
        let line = format!("{opening} {text}");
        let line = line.trim();
        let (mut to, mut text) = (Audience::Table, line);
        let breaks = line.match_indices(' ').map(|(at, _)| at).rev();
        for (words, rest) in breaks.map(|at| line.split_at(at)) {
            let id = gate::find_id(&ids, words).map_err(|detail| Refusal {
                detail: Some(detail),
                ..Reason::AmbiguousName.into()
            })?;
            if let Some(listener) = id {
                (to, text) = (Audience::Actor(listener.to_string()), rest.trim());
                break;
            }
        }
        Ok(self.append_speech(presence, from, Some(actor_id), Some(&to), text)?)
    }

    /// Append a GM's speech, spoken as `npc_id` when it is set. An `npc_id` that is not an NPC is
    /// `SCHEMA`. With no `audience`, the place `npc_id` stands in hears it.
    pub fn say_as_gm(
        &self,
        presence: &Presence,
        npc_id: Option<&ActorId>,
        audience: Option<&Audience>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        let from = match npc_id {
            None => Speaker::Gm,
            Some(id)
                if self.world.actors().get::<component::ControlType>(id).get()
                    == Some(component::ControlType::Npc) =>
            {
                Speaker::Npc(id.to_string())
            }
            Some(_) => return Err(Reason::Schema),
        };
        self.append_speech(presence, from, npc_id, audience, text)
    }

    /// Append one line of speech to the tape. Speech to an actor or a place must name one in
    /// the world, and a speaker's speech to a place must be where that speaker stands. With no
    /// `audience`, the speaker's place hears it.
    fn append_speech(
        &self,
        presence: &Presence,
        from: Speaker,
        speaker: Option<&ActorId>,
        audience: Option<&Audience>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        let text = check_speech_text(text)?.to_string();
        let actors = self.world.actors();
        let here = speaker.and_then(|speaker| actors.get::<component::Place>(speaker).get());
        let to = match audience {
            Some(Audience::Actor(actor)) if !actors.contains(&ActorId::from(actor.as_str())) => {
                return Err(Reason::Schema);
            }
            Some(Audience::Place(place)) if speaker.is_some() && here != Some(place.as_str()) => {
                return Err(Reason::Schema);
            }
            Some(Audience::Place(place))
                if !self.world.places().contains(&PlaceId::from(place.as_str())) =>
            {
                return Err(Reason::Schema);
            }
            Some(audience) => audience.clone(),
            None => Audience::Place(here.ok_or(Reason::Schema)?.to_string()),
        };
        let recipient_ids = speech_recipients(&self.world, presence, speaker, &to);
        let from_name = match speaker {
            Some(id) => self.canon().name(id.as_str()).ok_or(Reason::UnknownId)?,
            None => "",
        };
        let record = TapeRecord {
            seq: 0,
            from,
            from_name: from_name.to_string(),
            to,
            recipient_ids,
            text,
            round_index: self.world.round_index,
            world_time: self.world.world_time,
        };
        append_record(&self.root, record)
    }
}

impl<Lock> Session<Lock> {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn canon(&self) -> &Canon {
        self.world.canon()
    }

    pub fn pack_summary(&self) -> spookery_protocol::PackSummary {
        self.canon().to_pack_summary()
    }

    /// Every verb the pack's rules name: the commands a player seat may send as moves.
    pub fn verbs(&self) -> Vec<String> {
        self.canon().verbs()
    }

    /// Every PC in the cast.
    pub fn pcs(&self) -> impl Iterator<Item = &ActorId> {
        let actors = self.world.actors();
        let pcs = actors.query::<(ActorId, &component::ControlType)>();
        (pcs.iter())
            .filter(|(_, control_type)| matches!(control_type, component::ControlType::Pc))
            .map(|(id, _)| id)
    }

    /// The PC `text` names, by id or name, ignoring case and outer spaces. `None` when no PC
    /// has that id or name, and `AMBIGUOUS_NAME` when two do.
    pub fn find_pc(&self, text: &str) -> Result<Option<ActorId>, Reason> {
        let pcs = self.pcs().map(|id| {
            let name = self.canon().name(id.as_str()).ok_or(Reason::UnknownId)?;
            Ok(gate::NamedId::new(id.as_str(), name.to_string()))
        });
        let pcs = pcs.collect::<Result<Vec<_>, Reason>>()?;
        let id = gate::find_id(&pcs, text).map_err(|_| Reason::AmbiguousName)?;
        Ok(id.map(ActorId::from))
    }

    pub fn world(&self) -> &ecs::World {
        &self.world
    }

    /// Replace the ECS world after a replay. This does not write `save.yaml`.
    pub(crate) fn set_world(&mut self, world: ecs::World) {
        self.world = world;
    }

    /// Gate a player's line with `presence` seated, then queue it. The line is an intent
    /// from the start, so each refusal names its id. A typed name stands for the one id in
    /// sight that has it, and a name two of them share is `AMBIGUOUS_NAME`, its detail the
    /// name and those ids, as `Briar: bram, briar`. Only a save at
    /// the last round index has no next round to name, and refuses `SCHEMA` with an empty id.
    /// Returns the queued move and the target id it names.
    pub fn queue_player_move(
        &mut self,
        presence: &Presence,
        actor_id: &ActorId,
        verb: &str,
        target: Option<&str>,
    ) -> Result<(QueuedMove, Option<String>), gate::Refusal> {
        let words = target.filter(|token| !token.is_empty());
        let viewer = Viewer::Player(actor_id.clone());
        let ids = match words {
            Some(_) => gate::list_ids(self.canon(), &self.world, presence, &viewer),
            None => Vec::new(),
        };
        let match_id = match words {
            Some(words) => gate::find_id(&ids, words).map(|id| id.map(String::from)),
            None => Ok(None),
        };
        let id = match_id.clone().unwrap_or_default();
        let token = id.as_deref().or(words);
        let Ok(mut intent) = Intent::player(&self.world, actor_id, verb, token) else {
            return Err(gate::Refusal {
                id: String::new(),
                code: Reason::Schema,
                detail: None,
            });
        };
        if let Err(detail) = match_id {
            return Err(gate::Refusal {
                detail: Some(detail),
                ..gate::Refusal::new(&intent, Reason::AmbiguousName)
            });
        }
        if gate::is_cancel(&intent) {
            if token.is_some() || self.queue.iter().any(|queued| queued.id == intent.id) {
                return Err(gate::Refusal::new(&intent, Reason::Schema));
            }
            gate::admit_cancel(&Context::new(self.canon(), &self.world, presence, &intent))?;
        } else {
            let shaped = self.shape_line(&intent, token);
            let (rule, target) = shaped.map_err(|code| gate::Refusal::new(&intent, code))?;
            intent.target = target;
            let context = Context::new(self.canon(), &self.world, presence, &intent);
            gate::admit(&context, rule, Some(&ids))?;
        }
        let queued = QueuedMove {
            id: intent.id.clone(),
            actor_id: actor_id.to_string(),
            move_id: verb.to_string(),
        };
        let target = intent.target.token().map(String::from);
        self.queue_intent(intent);
        Ok((queued, target))
    }

    /// The rule a PC's line picks, and the target the line names for it. A line already
    /// queued is `SCHEMA`.
    fn shape_line(&self, line: &Intent, token: Option<&str>) -> Result<(&Rule, Target), Reason> {
        let Some(viewer) = &line.actor else {
            return Err(Reason::Schema);
        };
        let actors = self.world.actors();
        if !(actors.get::<component::ControlType>(viewer.actor_id())).is_pc() {
            return Err(Reason::Schema);
        }
        let (actor, verb) = (viewer, &line.move_id);
        let rule = gate::pick(self.canon(), &self.world, actor, line.source, verb, token)?;
        let target = target_for(rule, token)?;
        if self.queue.iter().any(|queued| queued.id == line.id) {
            return Err(Reason::Schema);
        }
        Ok((rule, target))
    }

    /// Pick the GM's intent's rule, admit it with `presence` seated, then queue it. A refusal
    /// is its code.
    pub fn queue_gm_intent(
        &mut self,
        presence: &Presence,
        spec: spookery_protocol::QueueArgs,
        new_id: impl FnOnce() -> Result<String, Reason>,
    ) -> Result<QueuedMove, Reason> {
        let actor_id: ActorId = spec
            .actor_id
            .filter(|id| !id.is_empty() && !id.contains(':'))
            .ok_or(Reason::Schema)?
            .into();
        let move_id = spec
            .move_id
            .filter(|m| !m.is_empty())
            .ok_or(Reason::Schema)?;
        let control = self.world.actors().get::<component::ControlType>(&actor_id);
        let viewer = control.viewer().ok_or(Reason::UnknownId)?;
        let token = spec.target.as_deref().filter(|t| !t.is_empty());
        let (canon, world) = (self.canon(), &self.world);
        let rule = gate::pick(canon, world, &viewer, Source::Gm, &move_id, token)?;
        let target = target_for(rule, token)?;
        let id = match spec.id {
            Some(id) if !id.is_empty() => id,
            Some(_) => return Err(Reason::Schema),
            None => new_id()?,
        };
        if self.queue.iter().any(|intent| intent.id == id) {
            return Err(Reason::Schema);
        }
        let round_index = match spec.round_index {
            Some(index) => index,
            None => self
                .world
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
            move_id: move_id.clone(),
            target,
            args,
            rationale_code: spec.rationale_code,
        };
        let context = Context::new(canon, world, presence, &intent);
        gate::admit(&context, rule, None).map_err(|refusal| refusal.code)?;
        self.queue_intent(intent);
        Ok(QueuedMove {
            id,
            actor_id: actor_id.into(),
            move_id,
        })
    }

    pub fn player_scrollback(&self, actor_id: &ActorId) -> Result<Vec<TapeRecord>, Reason> {
        read_scrollback(&self.root, actor_id.as_str())
    }

    pub fn budget(&self) -> &Budget {
        &self.canon().budget
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub(crate) fn queue_intent(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    pub fn perceive(
        &self,
        presence: &Presence,
        viewer_id: &str,
    ) -> Result<crate::rules::perceive::View, Reason> {
        crate::rules::perceive::perceive_budget(self, presence, viewer_id, PERCEIVE_BODY_CHARS)
    }

    /// The `help` commands for `actor_id`: what they can do now, and what they can name.
    pub fn help(
        &self,
        presence: &Presence,
        actor_id: &ActorId,
    ) -> Result<Vec<CommandView>, Reason> {
        crate::rules::lines::help(self.canon(), &self.world, presence, actor_id)
    }

    /// The `map` for `actor_id`: each place they visited, with the exits they saw there last.
    pub fn map(&self, actor_id: &ActorId) -> Result<Vec<PlaceView>, Reason> {
        crate::rules::lines::map(self.canon(), &self.world, actor_id)
    }

    pub fn perceive_trimmed(
        &self,
        presence: &Presence,
        viewer_id: &str,
        max_body_chars: usize,
    ) -> Result<crate::rules::perceive::View, Reason> {
        crate::rules::perceive::perceive_budget(self, presence, viewer_id, max_body_chars)
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
    /// `meter <entity> <id> <value> <max>`, the entity spelled as a rule's `of` names it.
    /// A working copy of the save with a round open on it under the pack's budget. A stage that
    /// refuses, or a commit that fails, drops the copy and the save stays as it was.
    pub(crate) fn begin_round(&self) -> ecs::World {
        let mut world = self.world.clone();
        world.open_round(&self.canon().budget);
        world
    }

    fn carried_meter_rows(&self) -> Vec<String> {
        let carried = self
            .canon()
            .meters
            .iter()
            .filter_map(|(id, def)| Some((def.carrier.clone()?, id)));
        let world_meters = self
            .canon()
            .world_meters
            .keys()
            .map(|id| (entity::Id::Global, id));
        carried
            .chain(world_meters)
            .filter_map(|(carrier, id)| {
                let meter = self.world.entities().get::<component::Meter>(&carrier);
                let value = meter.value(id).ok()?;
                let max = self.canon().meter_def(carrier.as_ref(), id)?.max;
                Some(format!("meter {carrier} {id} {value} {max}"))
            })
            .collect()
    }

    /// Effect rows for `outputs`, which end at this world's `round_index`, then the meter rows.
    pub fn digest(&self, outputs: &[crate::round::Output]) -> String {
        let start = self.world.round_index + 1 - outputs.len() as u64;
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

impl Session<()> {
    /// Open the pack at the journal tip and write nothing, so the pack lock is not needed.
    /// A read guard waits out a write in progress and drops once the session is built.
    /// A crash the writer has not recovered is read as it stands.
    ///
    /// A peeked session holds no lock, so it cannot play a round:
    ///
    /// ```compile_fail
    /// # let root = std::path::Path::new("pack");
    /// let presence = spookery_engine::Presence::default();
    /// spookery_engine::Session::peek(root).unwrap().play(&presence, "");
    /// ```
    pub fn peek(root: &Path) -> Result<Session<()>, Reason> {
        let _read = crate::store::lock::lock_read(root)?;
        crate::round::replay::peek_at_tip(root)
    }

    /// The host text at the journal tip: effect rows from the journal, then the meter rows.
    /// One read guard covers the session and the journal read, so both come from one save.
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
        let save =
            load_save_file(root, Some(&canon), &now_bytes).map_err(|diags| first_reason(&diags))?;
        Ok(Self {
            root: root.to_path_buf(),
            world: ecs::World::new(save, canon),
            queue: Vec::new(),
            lock: (),
        })
    }

    /// A session on an ECS world from memory, for replay. Nothing is checked: each journal
    /// record's hash proves the save it reaches.
    pub(crate) fn from_parts(pack: &Path, world: ecs::World) -> Self {
        Self {
            root: pack.to_path_buf(),
            world,
            queue: Vec::new(),
            lock: (),
        }
    }
}

/// Seated players who hear this speech. A seated speaker is included.
fn speech_recipients(
    world: &ecs::World,
    presence: &Presence,
    speaker: Option<&ActorId>,
    audience: &Audience,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    let seated = &presence.actors_present;
    match audience {
        Audience::Table => {
            ids.extend(seated.iter().map(ActorId::to_string));
        }
        Audience::Place(place) => {
            let here = world.actors_at(place);
            ids.extend(
                seated
                    .iter()
                    .filter(|id| here.contains(id))
                    .map(ActorId::to_string),
            );
        }
        Audience::Actor(actor) => {
            if seated.iter().any(|id| *id == *actor.as_str()) {
                ids.insert(actor.to_string());
            }
        }
    }
    if let Some(speaker) = speaker.filter(|speaker| seated.contains(speaker)) {
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

fn check_speech_text(text: &str) -> Result<&str, Reason> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > MAX_SAY_CHARS {
        return Err(Reason::Schema);
    }
    Ok(text)
}

/// Hash of the newest migrate in `live` past the checkpoint, or the save's own hash.
fn tip_pack_rev<'a>(live: &'a [Record], save: &'a Save) -> &'a str {
    live.iter()
        .rev()
        .filter(|entry| entry.state_rev_after() > save.state_rev)
        .find_map(|entry| match entry {
            Record::Migrate(record) => Some(record.pack_rev_after.as_str()),
            _ => None,
        })
        .unwrap_or(&save.pack_rev)
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
fn journal_continues(live: &[Record], save: &Save) -> bool {
    live.iter().any(|entry| match entry {
        Record::Checkpoint(_) => false,
        _ => entry.state_rev_after() > save.state_rev,
    })
}

/// The save in `bytes`, or every problem with it. A hand edit is refused unless a later
/// migrate record accepted it. A checkpoint save is not the tip, so only a save that no
/// record continues is checked against `canon`.
fn load_save_file(
    root: &Path,
    canon: Option<&Arc<Canon>>,
    bytes: &[u8],
) -> Result<Save, Vec<Diagnostic>> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        vec![schema_error(
            "state/save.yaml",
            "this file is not UTF-8 text",
            "save it as UTF-8",
        )
        .at_path("schema")]
    })?;
    let save = save::parse(text)?;
    let live = journal::read_live_records(root).unwrap_or_default();
    let mut problems = check_save_text(&save, bytes);
    if let Some(diag) = check_pack_rev(root, tip_pack_rev(&live, &save), bytes) {
        problems.insert(0, diag);
    }
    let migrate_follows = live.iter().any(|entry| {
        matches!(entry, Record::Migrate(_)) && entry.state_rev_after() > save.state_rev
    });
    let continues = journal_continues(&live, &save);
    let is_hand_edit = |diag: &Diagnostic| diag.code == Code::Error(Reason::HandEdit);
    if migrate_follows {
        problems.retain(|diag| !is_hand_edit(diag));
    } else if !problems.iter().any(is_hand_edit) {
        match journal::edits_head(root, save.state_rev, bytes) {
            Ok(false) => {}
            Ok(true) => problems.push(
                error(
                    "state/save.yaml",
                    Reason::HandEdit,
                    "this save differs from the checkpoint head it claims",
                    "restore the file, or run spook migrate to keep the edit if no round follows it",
                )
                .at_path("save"),
            ),
            Err(fault) => problems.push(
                error(
                    "state/journal/head.json",
                    fault.reason,
                    "the checkpoint head could not be read",
                    "check that state/journal/head.json is readable",
                )
                .at_path("schema"),
            ),
        }
    }
    if let Some(canon) = canon.filter(|_| !continues) {
        if let Err(more) = save::check_save(&ecs::World::new(save.clone(), Arc::clone(canon))) {
            problems.extend(more);
        }
    }
    if problems.is_empty() {
        Ok(save)
    } else {
        Err(problems)
    }
}

/// Read the pack and report every loader and lint problem. Writes nothing.
pub fn check_pack(root: &Path) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let canon = match load_canon(root) {
        Ok(canon) => Some(Arc::new(canon)),
        Err(more) => {
            diags.extend(more);
            None
        }
    };
    match journal::read_save(&journal::save_path(root)) {
        Ok(bytes) => {
            if let Err(more) = load_save_file(root, canon.as_ref(), &bytes) {
                diags.extend(more);
            }
        }
        Err(Reason::NotInSpec) => {}
        Err(reason) => diags.push(
            error(
                "state/save.yaml",
                reason,
                "state/save.yaml could not be read",
                "check that the file is readable",
            )
            .at_path("schema"),
        ),
    }
    if let Some(canon) = &canon {
        diags.extend(crate::canon::check_canon(canon));
    }
    diags
}

fn check_save_text(save: &Save, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    // Read as the file holds it: a save on its own has no canon to read its actors against.
    let saved = serde_json::to_value(save).unwrap_or_default();
    if saved["actors"]["control_type"].get("world").is_some() {
        let diag = schema_error(
            "state/save.yaml",
            "actor id world is reserved",
            "rename that actor",
        );
        diags.push(diag.at_path("actors.world"));
    }
    match save::emit(save) {
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
