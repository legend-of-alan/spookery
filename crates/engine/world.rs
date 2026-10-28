//! An open pack, and the check that reads one without writing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::canon::tick_settings::Budget;
use crate::canon::{check_save, load_canon, pack_rev, Canon};
use crate::diagnostic::{coded, locate_nth, reason_of, schema, Diagnostic, Severity};
use crate::effect::{payload_json, Effect};
use crate::error::Reason;
use crate::intent::{player_intent_id, Intent, IntentArgs, Source, Target, ViewerId};
use crate::journal::{self, JournalEntry};
use crate::model::{sort_unique, ActorLive, Now, Presence};
use crate::rule::{TargetKind, When};
use crate::rules::gate::{gate, verb_for, GateRefusal};
use crate::store::codec::{emit_save, load_save};
use crate::store::recover::recover;
use crate::store::tape::{append_record, read_session, read_tail, session_end_seq, TapeRecord};

use crate::draft::Draft;

/// Engine cap. A pack may define more PCs than this.
pub const MAX_PLAYER_SEATS: usize = 3;

/// One say, in characters. Four kibibytes still fits in an 8 KiB player line.
const SAY_TEXT_CAP: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Heard<'a> {
    Table,
    Place(&'a str),
    Actor(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuedMove {
    pub id: String,
    pub actor_id: String,
    pub move_name: String,
}

pub enum MoveOutcome {
    Queued(QueuedMove),
    Committed {
        advanced: bool,
        outputs: Vec<crate::RoundOutput>,
    },
    Busy {
        code: Reason,
        missing: Option<String>,
        outputs: Vec<crate::RoundOutput>,
    },
}

pub fn handle_player_move(
    world: &mut World,
    actor_id: &str,
    verb: &str,
    target: Option<&str>,
    gm_seated: bool,
    wall: &str,
) -> Result<MoveOutcome, GateRefusal> {
    let queued = match world.queue_player_move(actor_id, verb, target) {
        Ok(queued) => queued,
        Err(refusal) => {
            if !gm_seated && refusal.reason() == Reason::Busy {
                let outputs = world.commit_then_fast_forward(wall)?;
                return Ok(MoveOutcome::Busy {
                    code: refusal.reason(),
                    missing: refusal.missing().map(str::to_string),
                    outputs,
                });
            }
            return Err(refusal);
        }
    };
    if gm_seated {
        return Ok(MoveOutcome::Queued(queued));
    }
    let before = world.now().world_time;
    let outputs = world.commit_then_fast_forward(wall)?;
    Ok(MoveOutcome::Committed {
        advanced: world.now().world_time != before,
        outputs,
    })
}

pub struct GmIntentSpec {
    pub id: Option<String>,
    pub round_index: Option<u64>,
    pub actor_id: Option<String>,
    pub move_name: Option<String>,
    pub target: Option<String>,
    pub message_to: Option<Vec<String>>,
    pub rationale_code: String,
}

pub struct World {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
    pub(crate) now: Now,
    pub(crate) queue: Vec<Intent>,
    next_session: u64,
    warning: Option<&'static str>,
}

impl World {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn canon(&self) -> &Canon {
        &self.canon
    }

    pub fn pack_summary(&self) -> spookery_protocol::PackSummary {
        self.canon.pack_summary()
    }

    pub fn free_pcs(&self) -> Vec<String> {
        let present = &self.now.presence.actors_present;
        self.now
            .actors
            .iter()
            .filter(|(id, a)| a.is_pc() && !present.contains(id))
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn now(&self) -> &Now {
        &self.now
    }

    /// Replace the in-memory state after a replay. This does not write `save.yaml`.
    pub(crate) fn install_replayed(&mut self, now: Now) {
        self.now = now;
    }

    pub fn presence(&self) -> &Presence {
        &self.now.presence
    }

    /// Remember who is seated. This does not write the save.
    pub(crate) fn set_presence_inner(&mut self, mut presence: Presence) -> bool {
        sort_unique(&mut presence.actors_present);
        if presence == self.now.presence {
            return false;
        }
        self.now.presence = presence;
        true
    }

    /// The next seat id. Ids start at 1 and are not reused in this process.
    fn mint_session(&mut self) -> Result<String, Reason> {
        let id = format!("s{}", self.next_session);
        self.next_session = self.next_session.checked_add(1).ok_or(Reason::Schema)?;
        Ok(id)
    }

    pub fn seat_player(&mut self, actor_id: &str) -> Result<String, Reason> {
        let mut ids = self.now.presence.actors_present.clone();
        if ids.len() >= MAX_PLAYER_SEATS {
            return Err(Reason::SessionFull);
        }
        if ids.iter().any(|id| id == actor_id) {
            return Err(Reason::SeatTaken);
        }
        if !self.now.actors.get(actor_id).is_some_and(ActorLive::is_pc) {
            return Err(Reason::Schema);
        }
        ids.push(actor_id.to_string());
        self.commit_held(&ids)?;
        self.mint_session()
    }

    pub fn release_player_seat(
        &mut self,
        session_id: &str,
        actor_id: &str,
    ) -> Result<bool, Reason> {
        if session_id.is_empty() {
            return Err(Reason::Schema);
        }
        let mut ids = self.now.presence.actors_present.clone();
        ids.retain(|id| id != actor_id);
        self.commit_held(&ids)
    }

    pub fn clear_crashed(&mut self) -> Result<bool, Reason> {
        if self.now.presence.humans == 0 && self.now.presence.actors_present.is_empty() {
            return Ok(false);
        }
        Ok(self.set_presence_inner(Presence {
            humans: 0,
            actors_present: Vec::new(),
        }))
    }

    fn commit_held(&mut self, ids: &[String]) -> Result<bool, Reason> {
        Ok(self.set_presence_inner(Presence {
            humans: u32::try_from(ids.len()).map_err(|_| Reason::Schema)?,
            actors_present: ids.to_vec(),
        }))
    }

    pub fn queue_player_move(
        &mut self,
        actor_id: &str,
        verb: &str,
        target: Option<&str>,
    ) -> Result<QueuedMove, GateRefusal> {
        let actor = self
            .now
            .actors
            .get(actor_id)
            .filter(|actor| actor.is_pc())
            .ok_or(Reason::Schema)?;
        let target = self.verb_target(actor, verb, target, Reason::NotInSpec)?;
        let intent = self.player_intent(actor_id, verb, target)?;
        gate(&self.canon, &self.now, &intent)?;
        let queued = QueuedMove {
            id: intent.id.clone(),
            actor_id: actor_id.to_string(),
            move_name: verb.to_string(),
        };
        self.queue(intent);
        Ok(queued)
    }

    fn verb_target(
        &self,
        actor: &ActorLive,
        verb: &str,
        target: Option<&str>,
        missing: Reason,
    ) -> Result<Target, Reason> {
        let token = target.filter(|t| !t.is_empty());
        let rule = verb_for(
            &self.canon,
            Some(actor.place()),
            verb,
            token,
            actor.wielded(),
        )
        .ok_or(missing)?;
        let When::Verb { target: kind, .. } = &rule.when else {
            return Err(missing);
        };
        match (kind, token) {
            (TargetKind::None, None) => Ok(Target::Empty),
            (TargetKind::Actor | TargetKind::Item, Some(id)) => Ok(Target::Actor(id.to_string())),
            (TargetKind::Place | TargetKind::Exit, Some(id)) => Ok(Target::Place(id.to_string())),
            _ => Err(Reason::Schema),
        }
    }

    fn player_intent(&self, actor_id: &str, verb: &str, target: Target) -> Result<Intent, Reason> {
        let token = match &target {
            Target::Empty => "",
            Target::Actor(id) | Target::Place(id) => id.as_str(),
            Target::Clock(_) => return Err(Reason::Schema),
        };
        let round_index = self.now.round_index.checked_add(1).ok_or(Reason::Schema)?;
        let id = player_intent_id(
            &self.now.shard_id,
            round_index,
            actor_id,
            &canonical_line(verb, token),
        );
        if self.queue.iter().any(|intent| intent.id == id) {
            return Err(Reason::Schema);
        }
        Ok(Intent::new(
            id,
            round_index,
            ViewerId::Player(actor_id.to_string()),
            Source::Player,
            verb,
            target,
        ))
    }

    pub fn queue_gm_intent(
        &mut self,
        spec: GmIntentSpec,
        mint_id: impl FnOnce() -> Result<String, Reason>,
    ) -> Result<QueuedMove, Reason> {
        let actor_id = spec
            .actor_id
            .filter(|id| !id.is_empty() && !id.contains(':'))
            .ok_or(Reason::Schema)?;
        let move_name = spec
            .move_name
            .filter(|m| !m.is_empty())
            .ok_or(Reason::Schema)?;
        let actor = self.now.actors.get(&actor_id).ok_or(Reason::UnknownKey)?;
        let viewer = actor.viewer(&actor_id);
        let token = spec.target.as_deref().filter(|t| !t.is_empty());
        let is_npc = matches!(viewer, ViewerId::Npc(_));
        let is_habit = is_npc
            && self
                .canon
                .rules
                .habits_of(&actor_id)
                .iter()
                .any(|r| r.local_id == move_name);
        let target = match (is_habit, token) {
            (true, None) => Target::Empty,
            (true, Some(_)) => return Err(Reason::Schema),
            (false, _) if is_npc => {
                self.verb_target(actor, &move_name, token, Reason::UnknownMove)?
            }
            (false, _) => self.verb_target(actor, &move_name, token, Reason::NotInSpec)?,
        };
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
            None => self.now.round_index.checked_add(1).ok_or(Reason::Schema)?,
        };
        let args = IntentArgs {
            message_to: spec.message_to,
        };
        self.queue(Intent {
            id: id.clone(),
            round_index,
            actor: viewer,
            source: Source::Player,
            move_name: move_name.clone(),
            target,
            args,
            rationale_code: spec.rationale_code,
        });
        Ok(QueuedMove {
            id,
            actor_id,
            move_name,
        })
    }

    pub fn speak_player(
        &self,
        actor_id: &str,
        heard: Heard<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        if !self.now.actors.get(actor_id).is_some_and(ActorLive::is_pc) {
            return Err(Reason::Schema);
        }
        self.append_speech(&format!("player:{actor_id}"), Some(actor_id), heard, text)
    }

    pub fn speak_gm(&self, heard: Heard<'_>, text: &str) -> Result<TapeRecord, Reason> {
        self.append_speech("gm", None, heard, text)
    }

    fn append_speech(
        &self,
        from: &str,
        speaker: Option<&str>,
        heard: Heard<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        let text = one_line(text)?.to_string();
        if let Heard::Actor(actor) = heard {
            if !self.now.actors.contains_key(actor) {
                return Err(Reason::Schema);
            }
        }
        let (address, place, actor) = match heard {
            Heard::Table => ("table", None, None),
            Heard::Place(place) => ("place", Some(place.to_string()), None),
            Heard::Actor(actor) => ("actor", None, Some(actor.to_string())),
        };
        let recipients = speech_recipients(&self.now, speaker, heard);
        let round_index = self.now.round_index;
        let world_time = self.now.world_time;
        append_record(&self.root, |seq| TapeRecord {
            seq,
            from: from.to_string(),
            address: address.to_string(),
            place,
            actor,
            recipients,
            text,
            round_index,
            world_time,
        })
    }

    pub fn player_scrollback(&self, actor_id: &str) -> Result<Vec<TapeRecord>, Reason> {
        read_tail(&self.root, actor_id, 256)
    }

    pub fn tape_after(&self, since: u64) -> Result<Vec<TapeRecord>, Reason> {
        read_session(&self.root, since, None)
    }

    pub fn last_tape_seq(&self) -> Result<u64, Reason> {
        session_end_seq(&self.root)
    }

    pub fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub fn queue(&mut self, intent: Intent) {
        self.queue.push(intent);
    }

    /// A committed round whose checkpoint file was not finished. The host prints it.
    pub fn take_warning(&mut self) -> Option<&'static str> {
        self.warning.take()
    }

    pub(crate) fn note_warning(&mut self, text: &'static str) {
        self.warning = Some(text);
    }

    /// Open the pack at the journal tip. `save.yaml` stays the last checkpoint.
    /// Records after that checkpoint are in the live bundle, so closed bundles stay unread.
    pub fn resume(root: &Path) -> Result<Self, Reason> {
        crate::round::replay::open_at_tip(root)
    }

    pub fn play(&mut self, now_wall: &str) -> Result<crate::RoundOutput, Reason> {
        crate::round::play_round(
            self,
            now_wall,
            crate::round::play::RoundKind::Live {
                fast_forward: false,
            },
        )
    }

    /// Commit a queued round, then fast-forward while every seated PC is busy.
    /// An empty queue only fast-forwards.
    pub fn commit_then_fast_forward(
        &mut self,
        wall: &str,
    ) -> Result<Vec<crate::RoundOutput>, Reason> {
        let mut outputs = Vec::new();
        let committed = !self.queue.is_empty();
        if committed {
            outputs.push(self.play(wall)?);
        }
        let already = u32::from(committed);
        outputs.extend(crate::round::play_while_busy(self, wall, already)?);
        Ok(outputs)
    }

    pub fn catch_up(&mut self, now_wall: &str, downtime_hours: u64) -> Result<(), Reason> {
        crate::round::play_catch_up(self, now_wall, downtime_hours)
    }

    pub fn perceive(&self, as_arg: &str) -> Result<crate::perceive::View, Reason> {
        crate::perceive::perceive(self, as_arg)
    }

    pub fn perceive_budget(
        &self,
        as_arg: &str,
        budget_tokens: usize,
    ) -> Result<crate::perceive::View, Reason> {
        crate::perceive::perceive_budget(self, as_arg, budget_tokens)
    }

    pub fn help_lines(&self, actor_id: &str) -> String {
        crate::map::help_lines(&self.canon, &self.now, actor_id)
    }

    pub fn look_lines(&self, actor_id: &str) -> String {
        crate::map::look_lines(&self.canon, &self.now, actor_id)
    }

    pub fn player_lines(&self, actor_id: &str) -> String {
        crate::map::player_lines(&self.canon, &self.now, actor_id)
    }

    pub fn host_summary(&self) -> Result<String, Reason> {
        let entries = journal::read_verified(&self.root).map_err(|fault| fault.reason)?;
        let mut effects = Vec::new();
        for entry in &entries {
            let JournalEntry::Round(record) = entry else {
                continue;
            };
            for effect in &record.effects {
                effects.push((record.round_index, effect));
            }
        }
        Ok(self.format_host_lines(&effects))
    }

    /// Effect rows for `outputs` (ending at this world's `round_index`), then the clocks.
    pub fn host_lines(&self, outputs: &[crate::RoundOutput]) -> String {
        let start = self.now.round_index + 1 - outputs.len() as u64;
        let mut effects = Vec::new();
        for (offset, output) in outputs.iter().enumerate() {
            let round_index = start + offset as u64;
            for effect in &output.effects {
                effects.push((round_index, effect));
            }
        }
        self.format_host_lines(&effects)
    }

    fn format_host_lines(&self, effects: &[(u64, &Effect)]) -> String {
        let mut rows = Vec::new();
        for (round_index, effect) in effects {
            rows.push(format!(
                "effect {} {round_index} {}",
                effect.class_name(),
                payload_json(effect)
            ));
        }
        for (id, clock) in &self.now.clocks {
            let state = if clock.paused { "paused" } else { "running" };
            rows.push(format!("clock {id} {} {} {state}", clock.value, clock.max));
        }
        if rows.is_empty() {
            String::new()
        } else {
            rows.join("\n") + "\n"
        }
    }

    /// Open the pack. A save that later journal records continue is a checkpoint,
    /// and resume checks the tip instead.
    pub fn open(root: &Path) -> Result<Self, Reason> {
        if !root.join("pack.yaml").is_file() || !root.join("state/save.yaml").is_file() {
            return Err(Reason::NotInSpec);
        }
        recover(root)?;
        let canon = Arc::new(load_canon(root).map_err(reason_of)?);
        let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
        let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
        let now = load_save(now_text).map_err(reason_of)?;
        let mut problems = save_problems(&now, &now_bytes);
        if let Some(diag) = pack_rev_problem(root, &now, &now_bytes) {
            problems.insert(0, diag);
        }
        let continues = journal_continues(root, &now);
        if continues {
            problems.retain(|diag| diag.code != Severity::Error(Reason::HandEdit));
        }
        if !problems.is_empty() {
            return Err(reason_of(problems));
        }
        if !continues {
            check_save(&canon, &now).map_err(reason_of)?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            canon,
            now,
            queue: Vec::new(),
            next_session: 1,
            warning: None,
        })
    }

    /// The pack's canon with a save from memory, for replay. Nothing is checked:
    /// each journal record's hash proves the state it reaches.
    pub(crate) fn open_for_replay(pack: &Path, canon: Arc<Canon>, now: Now) -> Self {
        Self {
            root: pack.to_path_buf(),
            canon,
            now,
            queue: Vec::new(),
            next_session: 1,
            warning: None,
        }
    }
}

impl Draft {
    pub(crate) fn begin(world: &World) -> Self {
        Self::from_parts(
            Arc::clone(&world.canon),
            world.canon.budget,
            world.now.clone(),
        )
    }
}

/// Seated players who hear this say. The speaker is included.
fn speech_recipients(now: &Now, speaker: Option<&str>, heard: Heard<'_>) -> Vec<String> {
    let mut ids = BTreeSet::new();
    let seated = now.seated_pcs();
    match heard {
        Heard::Table => {
            ids.extend(seated);
        }
        Heard::Place(place) => {
            for id in seated {
                if now
                    .actors
                    .get(&id)
                    .is_some_and(|actor| actor.place() == place)
                {
                    ids.insert(id);
                }
            }
        }
        Heard::Actor(actor) => {
            if seated.iter().any(|id| id == actor) {
                ids.insert(actor.to_string());
            }
        }
    }
    if let Some(speaker) = speaker {
        ids.insert(speaker.to_string());
    }
    ids.into_iter().collect()
}

fn canonical_line(verb: &str, token: &str) -> String {
    if token.is_empty() {
        verb.to_string()
    } else {
        format!("{verb} {token}")
    }
}

fn one_line(text: &str) -> Result<&str, Reason> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > SAY_TEXT_CAP {
        return Err(Reason::Schema);
    }
    Ok(text)
}

/// Hash of the newest migrate past the checkpoint, or the save's own hash.
fn tip_pack_rev(root: &Path, now: &Now) -> String {
    let Ok(entries) = journal::read_live(root) else {
        return now.pack_rev.clone();
    };
    for entry in entries.iter().rev() {
        if entry.state_rev_after() <= now.state_rev {
            continue;
        }
        if let JournalEntry::Migrate(record) = entry {
            return record.pack_rev_after.clone();
        }
    }
    now.pack_rev.clone()
}

/// `Some` when the canon files are not the journal tip's hash.
fn pack_rev_problem(root: &Path, now: &Now, bytes: &[u8]) -> Option<Diagnostic> {
    let tip = tip_pack_rev(root, now);
    match pack_rev(root) {
        Ok(rev) if tip != rev => {
            let mut diag = coded(
                "state/save.yaml",
                Reason::PackRevMismatch,
                "pack_rev does not match the canon hash",
                "run spook migrate to store the current hash",
            )
            .with_path("pack_rev");
            if let Ok(text) = std::str::from_utf8(bytes) {
                if let Some((line, column)) = locate_nth(text, "pack_rev:", 0) {
                    diag = diag.at_line(line, Some(column));
                }
            }
            Some(diag)
        }
        Ok(_) => None,
        Err(_) => Some(
            coded(
                "state/save.yaml",
                Reason::Schema,
                "the canon files could not be hashed",
                "check that world/, cast/, and rules/ are readable",
            )
            .with_path("pack_rev"),
        ),
    }
}

/// The on-disk save is a checkpoint. Later journal records still have to be replayed.
fn journal_continues(root: &Path, now: &Now) -> bool {
    let Ok(entries) = journal::read_live(root) else {
        return false;
    };
    entries.iter().any(|entry| match entry {
        JournalEntry::Checkpoint(_) => false,
        _ => entry.state_rev_after() > now.state_rev,
    })
}

/// A migrate record past the checkpoint has already accepted the save's edit.
fn migrate_continues(root: &Path, now: &Now) -> bool {
    let Ok(entries) = journal::read_live(root) else {
        return false;
    };
    entries.iter().any(|entry| {
        matches!(entry, JournalEntry::Migrate(_)) && entry.state_rev_after() > now.state_rev
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
    let now_path = root.join("state/save.yaml");
    let loaded = if now_path.is_file() {
        match fs::read(&now_path) {
            Ok(bytes) => match std::str::from_utf8(&bytes) {
                Ok(text) => match load_save(text) {
                    Ok(now) => Some((bytes, now)),
                    Err(more) => {
                        diags.extend(more);
                        None
                    }
                },
                Err(_) => {
                    diags.push(
                        schema(
                            "state/save.yaml",
                            "this file is not UTF-8 text",
                            "save it as UTF-8",
                        )
                        .with_path("schema"),
                    );
                    None
                }
            },
            Err(_) => {
                diags.push(
                    coded(
                        "state/save.yaml",
                        Reason::Schema,
                        "state/save.yaml could not be read",
                        "check that the file is readable",
                    )
                    .with_path("schema"),
                );
                None
            }
        }
    } else {
        None
    };
    if let Some((bytes, now)) = &loaded {
        let mut problems = save_problems(now, bytes);
        let migrating = migrate_continues(root, now);
        if migrating {
            problems.retain(|diag| diag.code != Severity::Error(Reason::HandEdit));
        }
        if let Some(diag) = pack_rev_problem(root, now, bytes) {
            problems.insert(0, diag);
        }
        diags.extend(problems);
        // A checkpoint save is not the tip. Resume checks the tip when it opens.
        if let Some(canon) = canon.as_ref().filter(|_| !journal_continues(root, now)) {
            if let Err(more) = check_save(canon, now) {
                diags.extend(more);
            }
        }
    }
    if let Some(canon) = &canon {
        diags.extend(crate::canon::check_canon(
            canon,
            loaded.as_ref().map(|(_, now)| now),
        ));
    }
    diags
}

fn save_problems(now: &Now, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if now.actors.contains_key(crate::effect::WORLD) {
        diags.push(
            schema(
                "state/save.yaml",
                "actor id world is reserved",
                "rename that actor",
            )
            .with_path("actors.world"),
        );
    }
    match emit_save(now) {
        Ok(emitted) if emitted.as_bytes() != bytes => {
            let line = std::str::from_utf8(bytes)
                .ok()
                .and_then(|live| first_diff_line(live, &emitted).map(|(line_no, _)| line_no));
            let mut diag = coded(
                "state/save.yaml",
                Reason::HandEdit,
                "this save does not match the canonical text",
                "run spook migrate to keep the edit, or restore the file",
            );
            diag = match line {
                Some(line) => diag.at_line(line, None),
                None => diag.with_path("now"),
            };
            diags.push(diag);
        }
        Ok(_) => {}
        Err(_) => diags.push(
            coded(
                "state/save.yaml",
                Reason::Schema,
                "the save could not be written back as YAML",
                "fix the values so they match the now format",
            )
            .with_path("now"),
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
