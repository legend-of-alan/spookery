//! An open pack, and the moves and speech its seats send it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spookery_protocol::{CommandView, PlaceView};

use crate::canon::Budget;
use crate::canon::{load_canon, Canon};
use crate::diagnostic::first_reason;
use crate::ecs::id::{ActorId, PlaceId};
use crate::ecs::{self, component, Presence};
use crate::entity;
use crate::intent::{Intent, IntentArgs, Source, Target};
use crate::rules::context::Context;
use crate::rules::effect::{payload_json, Effect};
use crate::rules::gate;
use crate::rules::rule::{Rule, TargetKind};
use crate::store::journal;
use crate::store::records::Record;
use crate::store::tape::{append_record, read_scrollback, TapeRecord};
use crate::Reason;
use spookery_protocol::{Audience, QueuedMove, Refusal, Speaker};

/// One say, in characters. Four kibibytes still fits in an 8 KiB client message.
const MAX_SAY_CHARS: usize = 4 * 1024;

/// An open pack: its root, the ECS world its save reads into, the queued intents and the pack
/// lock `Lock`, which is `()` for a session that only reads.
pub struct Session<Lock = crate::PackGuard> {
    root: PathBuf,
    world: ecs::World,
    queue: Vec<Intent>,
    lock: Lock,
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
        } = crate::round::open::at_tip(root, &lock)?;
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

    /// The pack lock this session holds, which every write to `state/` needs.
    pub(crate) fn lock(&self) -> &crate::PackGuard {
        &self.lock
    }

    /// Play the queue with `presence` seated.
    pub fn play(
        &mut self,
        presence: &Presence,
        now_wall: &str,
    ) -> Result<crate::round::Output, Reason> {
        crate::round::play::run_round(self, presence, now_wall, false)
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
        crate::round::play::catch_up(self, now_wall, downtime_hours)
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
        let mut ids = gate::list_ids(self.canon(), &self.world, presence, actor_id);
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
        let here = speaker.and_then(|speaker| actors.get::<component::Place>(speaker).place_id());
        let to = match audience {
            Some(Audience::Actor(actor)) if !actors.contains(&ActorId::from(actor.as_str())) => {
                return Err(Reason::Schema);
            }
            Some(Audience::Place(place))
                if speaker.is_some() && here.is_none_or(|here| here != place.as_str()) =>
            {
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

    /// Accept `world`'s changes, swap it in and clear the queue, whose intents were aimed at
    /// the old world. The one place a session's world changes. This does not write `save.yaml`.
    pub(crate) fn replace_world(&mut self, mut world: ecs::World) {
        world.accept();
        self.world = world;
        self.queue.clear();
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
        let ids = match words {
            Some(_) => gate::list_ids(self.canon(), &self.world, presence, actor_id),
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
        let Some(actor_id) = &line.actor_id else {
            return Err(Reason::Schema);
        };
        let actors = self.world.actors();
        if !(actors.get::<component::ControlType>(actor_id)).is_pc() {
            return Err(Reason::Schema);
        }
        let verb = &line.move_id;
        let rule = gate::pick(
            self.canon(),
            &self.world,
            actor_id,
            line.source,
            verb,
            token,
        )?;
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
        if !self.world.actors().contains(&actor_id) {
            return Err(Reason::UnknownId);
        }
        let token = spec.target.as_deref().filter(|t| !t.is_empty());
        let (canon, world) = (self.canon(), &self.world);
        let rule = gate::pick(canon, world, &actor_id, Source::Gm, &move_id, token)?;
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
            actor_id: Some(actor_id.clone()),
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
        crate::rules::perceive::build_view_by_id(self, presence, viewer_id)
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
            .filter_map(|(id, desc)| Some((desc.carrier.clone()?, id)));
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
                let max = self.canon().meter_desc(carrier.as_ref(), id)?.max;
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
        let _read = crate::store::lock::take_read(root)?;
        Ok(crate::round::open::peek_at_tip(root)?)
    }

    /// The host text at the journal tip: effect rows from the journal, then the meter rows.
    /// One read guard covers the session and the journal read, so both come from one save.
    pub fn peek_digest(root: &Path) -> Result<String, Reason> {
        let _read = crate::store::lock::take_read(root)?;
        crate::round::open::peek_at_tip(root)?.read_digest()
    }

    /// Open the pack. A save that later journal records continue is a checkpoint,
    /// and resume checks the tip instead.
    pub fn open(root: &Path) -> Result<Self, Reason> {
        if !root.join("pack.yaml").is_file() {
            return Err(Reason::NotInSpec);
        }
        let now_bytes = journal::read_save(&journal::save_path(root))?;
        let canon = Arc::new(load_canon(root).map_err(|diags| first_reason(&diags))?);
        let save = crate::check::load_save_file(root, Some(&canon), &now_bytes)
            .map_err(|diags| first_reason(&diags))?;
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
    match (rule.when.target_kind(), token) {
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
