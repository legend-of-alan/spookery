//! An open pack, and the check that reads one without writing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::canon::Budget;
use crate::canon::{check_save, load_canon, pack_rev, Canon};
use crate::diagnostic::{error, first_reason, locate_nth, schema_error, Code, Diagnostic};
use crate::effect::{payload_json, Effect};
use crate::error::Reason;
use crate::intent::{player_intent_id, Intent, IntentArgs, Source, Target, Viewer};
use crate::journal::{self, Record};
use crate::model::{ActorLive, Presence, State};
use crate::rule::{TargetKind, When};
use crate::rules::gate::{gate, verb_for, GateRefusal};
use crate::store::codec::{emit_save, parse_save};
use crate::store::tape::{append_record, last_seq, read_after, read_tail, TapeRecord};
use crate::{IntentCall, QueuedMove};

use crate::draft::Draft;

/// One say, in characters. Four kibibytes still fits in an 8 KiB player line.
const MAX_SAY_CHARS: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Address<'a> {
    Table,
    Place(&'a str),
    Actor(&'a str),
}

pub struct World<L = crate::PackGuard> {
    pub(crate) root: PathBuf,
    canon: Arc<Canon>,
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
            state: now,
            queue,
            warning,
            lock: (),
        } = crate::round::replay::open_at_tip(root, &lock)?;
        Ok(World {
            root,
            canon,
            state: now,
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
        actor_id: &str,
        heard: Address<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        if !self
            .state
            .actors
            .get(actor_id)
            .is_some_and(ActorLive::is_pc)
        {
            return Err(Reason::Schema);
        }
        let from = format!("player:{actor_id}");
        self.append_speech(presence, &from, Some(actor_id), heard, text)
    }

    pub fn say_as_gm(
        &self,
        presence: &Presence,
        heard: Address<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        self.append_speech(presence, "gm", None, heard, text)
    }

    fn append_speech(
        &self,
        presence: &Presence,
        from: &str,
        speaker: Option<&str>,
        heard: Address<'_>,
        text: &str,
    ) -> Result<TapeRecord, Reason> {
        let text = check_say_text(text)?.to_string();
        if let Address::Actor(actor) = heard {
            if !self.state.actors.contains_key(actor) {
                return Err(Reason::Schema);
            }
        }
        let (address, place, actor) = match heard {
            Address::Table => ("table", None, None),
            Address::Place(place) => ("place", Some(place.to_string()), None),
            Address::Actor(actor) => ("actor", None, Some(actor.to_string())),
        };
        let recipients = speech_recipients(&self.state, presence, speaker, heard);
        let round_index = self.state.round_index;
        let world_time = self.state.world_time;
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

    /// PCs that nobody in `presence` holds.
    pub fn free_pcs(&self, presence: &Presence) -> Vec<String> {
        let present = &presence.actors_present;
        self.state
            .actors
            .iter()
            .filter(|(id, a)| a.is_pc() && !present.contains(id))
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    /// Replace the in-memory state after a replay. This does not write `save.yaml`.
    pub(crate) fn set_state(&mut self, now: State) {
        self.state = now;
    }

    /// Gate a player's move with `presence` seated, then queue it.
    pub fn queue_player_move(
        &mut self,
        presence: &Presence,
        actor_id: &str,
        verb: &str,
        target: Option<&str>,
    ) -> Result<QueuedMove, GateRefusal> {
        let actor = self
            .state
            .actors
            .get(actor_id)
            .filter(|actor| actor.is_pc())
            .ok_or(Reason::Schema)?;
        let target = self.verb_target(actor, verb, target, Reason::NotInSpec)?;
        let intent = self.player_intent(actor_id, verb, target)?;
        gate(&self.canon, &self.state, presence, &intent)?;
        let queued = QueuedMove {
            id: intent.id.clone(),
            actor_id: actor_id.to_string(),
            move_name: verb.to_string(),
        };
        self.queue_intent(intent);
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
            (TargetKind::None, Some(_)) | (_, None) => Err(Reason::Schema),
            (kind, Some(id)) => Ok(Target::new(*kind, id)),
        }
    }

    fn player_intent(&self, actor_id: &str, verb: &str, target: Target) -> Result<Intent, Reason> {
        let token = match &target {
            Target::Empty => "",
            Target::Actor(id) | Target::Item(id) | Target::Place(id) | Target::Exit(id) => {
                id.as_str()
            }
            Target::Clock(_) => return Err(Reason::Schema),
        };
        let round_index = self
            .state
            .round_index
            .checked_add(1)
            .ok_or(Reason::Schema)?;
        let id = player_intent_id(
            &self.state.world_id,
            round_index,
            actor_id,
            &join_move(verb, token),
        );
        if self.queue.iter().any(|intent| intent.id == id) {
            return Err(Reason::Schema);
        }
        Ok(Intent::new(
            id,
            round_index,
            Viewer::Player(actor_id.to_string()),
            Source::Player,
            verb,
            target,
        ))
    }

    pub fn queue_gm_intent(
        &mut self,
        spec: IntentCall,
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
        let actor = self.state.actors.get(&actor_id).ok_or(Reason::UnknownId)?;
        let viewer = actor.viewer(&actor_id);
        let token = spec.target.as_deref().filter(|t| !t.is_empty());
        let is_npc = matches!(viewer, Viewer::Npc(_));
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
            None => self
                .state
                .round_index
                .checked_add(1)
                .ok_or(Reason::Schema)?,
        };
        let args = IntentArgs {
            message_to: spec.message_to,
        };
        self.queue_intent(Intent {
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

    pub fn player_scrollback(&self, actor_id: &str) -> Result<Vec<TapeRecord>, Reason> {
        read_tail(&self.root, actor_id, 256)
    }

    pub fn tape_after(&self, since: u64) -> Result<Vec<TapeRecord>, Reason> {
        read_after(&self.root, since, None)
    }

    pub fn last_tape_seq(&self) -> Result<u64, Reason> {
        last_seq(&self.root)
    }

    pub fn budget(&self) -> &Budget {
        &self.canon.budget
    }

    pub fn queued_intents(&self) -> &[Intent] {
        &self.queue
    }

    pub fn queue_intent(&mut self, intent: Intent) {
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
    ) -> Result<crate::perceive::View, Reason> {
        crate::perceive::perceive(self, presence, as_arg)
    }

    pub fn perceive_trimmed(
        &self,
        presence: &Presence,
        as_arg: &str,
        max_body_chars: usize,
    ) -> Result<crate::perceive::View, Reason> {
        crate::perceive::perceive_budget(self, presence, as_arg, max_body_chars)
    }

    pub fn help_lines(&self, presence: &Presence, actor_id: &str) -> String {
        crate::travel::help_lines(&self.canon, &self.state, presence, actor_id)
    }

    pub fn look_lines(&self, presence: &Presence, actor_id: &str) -> String {
        crate::travel::look_lines(&self.canon, &self.state, presence, actor_id)
    }

    pub fn map_lines(&self, presence: &Presence, actor_id: &str) -> String {
        crate::travel::map_lines(&self.canon, &self.state, presence, actor_id)
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

    /// Effect rows for `outputs` (ending at this world's `round_index`), then the clocks.
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
                effect.kind_name(),
                payload_json(effect)
            ));
        }
        for (id, clock) in &self.state.clocks {
            let state = if clock.paused { "paused" } else { "running" };
            rows.push(format!("clock {id} {} {} {state}", clock.value, clock.max));
        }
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

    /// The host text at the journal tip: effect rows from the journal, then the clocks.
    /// One read guard covers the World and the journal read, so both come from one state.
    pub fn peek_digest(root: &Path) -> Result<String, Reason> {
        let _read = crate::store::lock::lock_read(root)?;
        crate::round::replay::peek_at_tip(root)?.read_digest()
    }

    /// Open the pack. A save that later journal records continue is a checkpoint,
    /// and resume checks the tip instead.
    pub fn open(root: &Path) -> Result<Self, Reason> {
        if !root.join("pack.yaml").is_file() || !root.join("state/save.yaml").is_file() {
            return Err(Reason::NotInSpec);
        }
        let canon = Arc::new(load_canon(root).map_err(|diags| first_reason(&diags))?);
        let now_bytes = fs::read(root.join("state/save.yaml")).map_err(|_| Reason::NotInSpec)?;
        let now_text = std::str::from_utf8(&now_bytes).map_err(|_| Reason::Schema)?;
        let now = parse_save(now_text).map_err(|diags| first_reason(&diags))?;
        let mut problems = save_problems(&now, &now_bytes);
        if let Some(diag) = pack_rev_problem(root, &now, &now_bytes) {
            problems.insert(0, diag);
        }
        let continues = journal_continues(root, &now);
        if continues {
            problems.retain(|diag| diag.code != Code::Error(Reason::HandEdit));
        }
        if !problems.is_empty() {
            return Err(first_reason(&problems));
        }
        if !continues {
            check_save(&canon, &now).map_err(|diags| first_reason(&diags))?;
        }
        Ok(Self {
            root: root.to_path_buf(),
            canon,
            state: now,
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

impl<'p> Draft<'p> {
    pub(crate) fn begin<L>(world: &World<L>, presence: &'p Presence) -> Self {
        Self::from_parts(
            Arc::clone(&world.canon),
            world.canon.budget,
            world.state.clone(),
            presence,
        )
    }
}

/// Seated players who hear this say. The speaker is included.
fn speech_recipients(
    now: &State,
    presence: &Presence,
    speaker: Option<&str>,
    heard: Address<'_>,
) -> Vec<String> {
    let mut ids = BTreeSet::new();
    let seated = &presence.actors_present;
    match heard {
        Address::Table => {
            ids.extend(seated.iter().cloned());
        }
        Address::Place(place) => {
            for id in seated {
                if now
                    .actors
                    .get(id)
                    .is_some_and(|actor| actor.place() == place)
                {
                    ids.insert(id.clone());
                }
            }
        }
        Address::Actor(actor) => {
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

fn join_move(verb: &str, token: &str) -> String {
    if token.is_empty() {
        verb.to_string()
    } else {
        format!("{verb} {token}")
    }
}

fn check_say_text(text: &str) -> Result<&str, Reason> {
    if text.is_empty() || text.contains('\n') || text.chars().count() > MAX_SAY_CHARS {
        return Err(Reason::Schema);
    }
    Ok(text)
}

/// Hash of the newest migrate past the checkpoint, or the save's own hash.
fn tip_pack_rev(root: &Path, now: &State) -> String {
    let Ok(entries) = journal::read_live_records(root) else {
        return now.pack_rev.clone();
    };
    for entry in entries.iter().rev() {
        if entry.state_rev_after() <= now.state_rev {
            continue;
        }
        if let Record::Migrate(record) = entry {
            return record.pack_rev_after.clone();
        }
    }
    now.pack_rev.clone()
}

/// `Some` when the canon files are not the journal tip's hash.
fn pack_rev_problem(root: &Path, now: &State, bytes: &[u8]) -> Option<Diagnostic> {
    let tip = tip_pack_rev(root, now);
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

/// The on-disk save is a checkpoint. Later journal records still have to be replayed.
fn journal_continues(root: &Path, now: &State) -> bool {
    let Ok(entries) = journal::read_live_records(root) else {
        return false;
    };
    entries.iter().any(|entry| match entry {
        Record::Checkpoint(_) => false,
        _ => entry.state_rev_after() > now.state_rev,
    })
}

/// A migrate record past the checkpoint has already accepted the save's edit.
fn migrate_continues(root: &Path, now: &State) -> bool {
    let Ok(entries) = journal::read_live_records(root) else {
        return false;
    };
    entries
        .iter()
        .any(|entry| matches!(entry, Record::Migrate(_)) && entry.state_rev_after() > now.state_rev)
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
                Ok(text) => match parse_save(text) {
                    Ok(now) => Some((bytes, now)),
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
            Err(_) => {
                diags.push(
                    error(
                        "state/save.yaml",
                        Reason::Schema,
                        "state/save.yaml could not be read",
                        "check that the file is readable",
                    )
                    .at_path("schema"),
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
            problems.retain(|diag| diag.code != Code::Error(Reason::HandEdit));
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

fn save_problems(now: &State, bytes: &[u8]) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    if now.actors.contains_key(crate::effect::WORLD) {
        diags.push(
            schema_error(
                "state/save.yaml",
                "actor id world is reserved",
                "rename that actor",
            )
            .at_path("actors.world"),
        );
    }
    match emit_save(now) {
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
