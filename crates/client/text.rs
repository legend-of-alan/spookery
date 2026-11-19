//! Readable text for player and GM messages, the picker, and the status and commit replies.

use std::collections::BTreeMap;

use crate::protocol::{
    self, ChangeValue, ChangeView, CommandView, CommitBody, ExitView, ItemStack, MeterView,
    NoteView, Outcome, PlaceView, Roll, ServerBody, Speaker, Speech, TableStatus,
};

/// The picker's heading.
pub(crate) const PICKER_HEADING: &str = "Choose a character";

/// The line under the picker when every PC is held.
pub(crate) const NONE_FREE: &str = "None is free.";

/// One PC of the picker: its name as the hello gives it, and its id while nobody holds it.
pub(crate) struct PickerRow {
    pub(crate) name: String,
    pub(crate) open_id: Option<String>,
}

impl PickerRow {
    /// `Rowan`, or `Rowan (in use)` for a PC someone holds.
    pub(crate) fn label(&self) -> String {
        match self.open_id {
            Some(_) => self.name.clone(),
            None => format!("{} (in use)", self.name),
        }
    }
}

/// Every PC a hello lists, open and taken, sorted by label.
pub(crate) fn picker_rows(
    open: &[String],
    taken: &[String],
    pc_names: &BTreeMap<String, String>,
) -> Vec<PickerRow> {
    let name = |id: &String| pc_names.get(id).cloned().unwrap_or_default();
    let open = open.iter().map(|id| PickerRow {
        name: name(id),
        open_id: Some(id.clone()),
    });
    let taken = taken.iter().map(|id| PickerRow {
        name: name(id),
        open_id: None,
    });
    let mut rows: Vec<PickerRow> = open.chain(taken).collect();
    rows.sort_by_key(PickerRow::label);
    rows
}

/// The picker as lines mode prints it: the heading, each row, and `None is free.` when every
/// PC is held.
pub(crate) fn render_picker(rows: &[PickerRow]) -> String {
    let labels: Vec<String> = rows.iter().map(PickerRow::label).collect();
    let mut text = format!("{PICKER_HEADING}:\n{}", labels.join("\n"));
    if rows.iter().all(|row| row.open_id.is_none()) {
        text.push('\n');
        text.push_str(NONE_FREE);
    }
    text
}

/// Readable text for one player body. Empty means print nothing.
pub fn render(body: &ServerBody) -> String {
    match body {
        ServerBody::Say(speech) => render_speech(speech),
        ServerBody::Queued { .. } => "Your action is queued.".to_string(),
        ServerBody::Check { outcome } => match outcome {
            Outcome::Success => "Your roll succeeded.".to_string(),
            Outcome::Partial => "Your roll partly succeeded.".to_string(),
            Outcome::Miss => "Your roll missed.".to_string(),
        },
        ServerBody::Resume { notes } => render_resume(notes),
        ServerBody::Lines { text } => text.clone(),
        ServerBody::Changes { viewer, changes } => render_changes(viewer, changes),
        ServerBody::Refused(refusal) => refusal.to_string(),
        ServerBody::View(view) => render_view(view),
        ServerBody::Help { commands } => render_help(commands),
        ServerBody::Map { places } => render_map(places),
        ServerBody::Claimed { scrollback, .. } => render_scrollback(scrollback),
        ServerBody::Hello { .. }
        | ServerBody::Released
        | ServerBody::Presence { .. }
        | ServerBody::Round { .. }
        | ServerBody::Committed(_)
        | ServerBody::Status(_) => String::new(),
    }
}

fn render_speech(speech: &Speech) -> String {
    format!("{}: {}", speaker_for_player(speech), speech.text)
}

/// The name a player reads for who spoke: `You` for a player, else `GM` or the actor's name.
pub(crate) fn speaker_for_player(speech: &Speech) -> &str {
    match &speech.from {
        Speaker::Player(_) => "You",
        Speaker::Gm | Speaker::Npc(_) => speaker_name(speech),
    }
}

/// The speech a claim replays, under an `Earlier:` heading.
fn render_scrollback(scrollback: &[Speech]) -> String {
    if scrollback.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = scrollback.iter().map(render_speech).collect();
    format!(
        "Earlier:
{}",
        lines.join(
            "
"
        )
    )
}

/// Turns the messages of one sitting into readable text.
#[derive(Default)]
pub struct Renderer {
    seen_resume: bool,
    after_scrollback: bool,
}

impl Renderer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Text for this body. Empty means print nothing.
    /// A leading newline is the blank line between the scrollback and the first live text.
    pub fn render(&mut self, body: &ServerBody) -> String {
        match body {
            ServerBody::Claimed { scrollback, .. } => {
                self.after_scrollback = !scrollback.is_empty();
                render(body)
            }
            ServerBody::Resume { notes } => {
                if self.seen_resume {
                    return String::new();
                }
                self.seen_resume = true;
                self.after_scrollback(render_resume(notes))
            }
            _ => self.after_scrollback(render(body)),
        }
    }

    /// Live text, after a blank line when it is the first after the scrollback.
    fn after_scrollback(&mut self, text: String) -> String {
        if text.is_empty() || !self.after_scrollback {
            return text;
        }
        self.after_scrollback = false;
        format!(
            "
{text}"
        )
    }
}

fn render_resume(notes: &[NoteView]) -> String {
    let lines: Vec<&str> = notes
        .iter()
        .map(|note| note.text.trim())
        .filter(|text| !text.is_empty())
        .collect();
    if lines.is_empty() {
        String::new()
    } else {
        format!("While you were away:\n{}", lines.join("\n"))
    }
}

pub fn render_view(view: &protocol::View) -> String {
    let protocol::View {
        viewer,
        place: _,
        place_name,
        body,
        actor_names,
        cant_act,
        busy,
        visible_actors,
        known_facts,
        notes,
        items,
        meters,
        world_meters,
        exits,
        unknown_mentions: _,
        habits: _,
    } = view;
    let mut sections = Vec::new();
    if !place_name.is_empty() {
        sections.push(format!("You are in {place_name}."));
    }
    for line in [cant_act, busy] {
        if !line.is_empty() {
            sections.push(line.clone());
        }
    }
    let prose = body.trim();
    if !prose.is_empty() {
        sections.push(prose.to_string());
    }
    if !visible_actors.is_empty() {
        let names = visible_actors
            .iter()
            .zip(actor_names)
            .map(|(id, name)| if id == viewer { "You" } else { name })
            .collect::<Vec<_>>()
            .join(", ");
        sections.push(format!("Here: {names}"));
    }
    push_lines(
        &mut sections,
        "You know:",
        known_facts.iter().map(|fact| fact.text.as_str()),
    );
    push_lines(
        &mut sections,
        "Notes:",
        notes.iter().map(|note| note.text.as_str()),
    );
    if !items.is_empty() {
        let lines: Vec<String> = items.iter().map(item_line).collect();
        sections.push(format!("Items:\n{}", lines.join("\n")));
    }
    for (title, meters) in [("Meters:", meters), ("The world:", world_meters)] {
        if !meters.is_empty() {
            let lines: Vec<String> = meters.iter().map(meter_line).collect();
            sections.push(format!("{title}\n{}", lines.join("\n")));
        }
    }
    sections.push(exit_section(exits));
    sections.join("\n\n")
}

/// One command per line: `look`, `ask Briar`, or `strike <actor>: Briar, Aldric`.
pub fn render_help(commands: &[CommandView]) -> String {
    let lines: Vec<String> = commands.iter().map(command_line).collect();
    lines.join("\n")
}

fn command_line(command: &CommandView) -> String {
    let id = &command.id;
    if let Some(owner) = &command.owner {
        let owner = command.names.first().unwrap_or(owner);
        return format!("{id} {owner}");
    }
    if command.target == "none" {
        return id.clone();
    }
    let line = format!("{id} <{}>", command.target);
    if command.names.is_empty() {
        line
    } else {
        format!("{line}: {}", command.names.join(", "))
    }
}

/// One sentence per change it can word, on one line. Other components print nothing.
fn render_changes(viewer: &str, changes: &[ChangeView]) -> String {
    let sentences: Vec<String> = changes
        .iter()
        .filter_map(|change| {
            let whose = render_possessive(viewer, change);
            match (
                change.component_id.as_str(),
                as_number(change.after.as_ref()),
            ) {
                ("meter", Some(after)) => {
                    Some(format!("{whose} {} is now {after}.", change.entry_id))
                }
                ("item", Some(_)) => render_item_change(viewer, change, changes),
                ("place", _) if !change.entry_name.is_empty() => {
                    Some(format!("{whose} place is now {}.", change.entry_name))
                }
                _ => None,
            }
        })
        .collect();
    sentences.join(" ")
}

/// A change's value when it is a number: a meter's reading or an item's count.
fn as_number(value: Option<&ChangeValue>) -> Option<i64> {
    value?.as_number()
}

/// A change to what an actor carries or what lies in the viewer's place. A place's change that
/// matches an actor's is told by the actor's sentence.
fn render_item_change(viewer: &str, change: &ChangeView, changes: &[ChangeView]) -> Option<String> {
    let after = as_number(change.after.as_ref())?;
    let lost = after < as_number(change.before.as_ref())?;
    let name = &change.entry_name;
    let other_side = changes.iter().find(|other| {
        other.component_id == "item"
            && other.entry_id == change.entry_id
            && (as_number(other.before.as_ref()).zip(as_number(other.after.as_ref())))
                .is_some_and(|(before, after)| (after < before) != lost)
    });
    let other_is_actor = other_side.map(|other| is_actor(&other.entity_id));
    if !is_actor(&change.entity_id) {
        return match (other_is_actor, lost) {
            (Some(true), _) => None,
            (_, true) => Some(format!("The {name} here is gone.")),
            (_, false) => Some(format!("A {name} turned up here.")),
        };
    }
    let who = if change.entity_id == viewer {
        "You"
    } else {
        &change.entity_name
    };
    let whose = render_possessive(viewer, change);
    let sentence = match (lost, other_is_actor) {
        (true, Some(true)) => format!("{who} handed over the {name}."),
        (true, Some(false)) => format!("{who} dropped the {name}."),
        (false, Some(true)) => format!("{who} received the {name}."),
        (false, Some(false)) => format!("{who} picked up the {name}."),
        (true, None) if after == 0 => format!("{whose} {name} was used up."),
        (true, None) => format!("{whose} {name} was lost."),
        (false, None) => format!("{whose} {name} was gained."),
    };
    Some(sentence)
}

/// `Your`, `Briar's`, `Gate Yard's`, `The strain story's` or `The world's`.
fn render_possessive(viewer: &str, change: &ChangeView) -> String {
    let name = &change.entity_name;
    if change.entity_id == viewer {
        "Your".to_string()
    } else if change.entity_id == "world" {
        "The world's".to_string()
    } else if change.entity_id.starts_with("story:") {
        format!("The {name} story's")
    } else {
        format!("{name}'s")
    }
}

/// Whether a change's entity is an actor: an actor id has no `place:` or `story:` prefix.
fn is_actor(entity_id: &str) -> bool {
    entity_id != "world" && !entity_id.contains(':')
}

/// Each place, then its exits indented under it.
pub fn render_map(places: &[PlaceView]) -> String {
    let mut lines = Vec::new();
    for place in places {
        lines.push(place.name.clone());
        for exit in &place.exits {
            lines.push(format!("  {}", exit_phrase(exit)));
        }
    }
    lines.join("\n")
}

/// `stairs (locked) — 2 ticks`, or `Docks Street — 0 ticks` for an exit named by where it
/// goes.
pub(crate) fn exit_phrase(exit: &ExitView) -> String {
    let (name, ticks) = (&exit.name, exit.ticks);
    if exit.locked {
        format!("{name} (locked) — {ticks} ticks")
    } else {
        format!("{name} — {ticks} ticks")
    }
}

/// `Exits:` and one exit per line, or `none`.
fn exit_section(exits: &[ExitView]) -> String {
    if exits.is_empty() {
        return "Exits:\nnone".to_string();
    }
    let lines: Vec<String> = exits.iter().map(exit_phrase).collect();
    format!("Exits:\n{}", lines.join("\n"))
}

fn push_lines<'a>(sections: &mut Vec<String>, title: &str, lines: impl Iterator<Item = &'a str>) {
    let lines = lines
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return;
    }
    sections.push(format!("{title}\n{}", lines.join("\n")));
}

/// `Lamp oil: 4`: the item's name and count.
pub(crate) fn item_line(item: &ItemStack) -> String {
    format!("{}: {}", item.name, item.count)
}

/// `Hp: 8/16`: the meter's name, value and max.
pub(crate) fn meter_line(meter: &MeterView) -> String {
    format!("{}: {}/{}", humanize(&meter.id), meter.value, meter.max)
}

/// `docks_shop` becomes "Docks shop".
pub(crate) fn humanize(id: &str) -> String {
    let spaced = id.replace('_', " ");
    let mut chars = spaced.chars();
    let Some(first) = chars.next() else {
        return spaced;
    };
    let mut titled = String::new();
    titled.push(first.to_ascii_uppercase());
    titled.push_str(chars.as_str());
    titled
}

pub fn render_status(status: &TableStatus) -> String {
    let present = &status.presence.actors_present;
    let seated = if present.is_empty() {
        "Seated: nobody".to_string()
    } else {
        let names = present
            .iter()
            .map(|id| humanize(id))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Seated: {names}")
    };
    let queued = if status.queued.is_empty() {
        "Queue: empty".to_string()
    } else {
        let lines = status
            .queued
            .iter()
            .map(|row| format!("{}: {}", humanize(&row.actor_id), row.move_id))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Queue:\n{lines}")
    };
    format!("{seated}\n{queued}")
}

/// What a commit changed, then each roll and dice line. The reply is the one place a GM sees
/// this commit's rolls.
fn render_commit(body: &CommitBody) -> String {
    let mut lines = vec![if body.changed_keys.is_empty() {
        "Nothing changed.".to_string()
    } else {
        format!("Changed:\n{}", body.changed_keys.join("\n"))
    }];
    lines.extend(body.rolls.iter().map(render_check));
    lines.extend(body.dice.iter().map(|line| format!("dice {line}")));
    lines.join("\n")
}

/// What a refusal prints on stderr: the code in raw mode, the sentence in text mode.
pub(crate) fn render_refusal(refusal: &protocol::Refusal, raw: bool) -> String {
    if raw {
        refusal.code.as_str().to_string()
    } else {
        refusal.to_string()
    }
}

/// The name speech shows for who spoke: `GM`, or the speaking actor's name.
fn speaker_name(speech: &Speech) -> &str {
    match speech.from {
        Speaker::Gm => "GM",
        Speaker::Player(_) | Speaker::Npc(_) => &speech.from_name,
    }
}

/// Readable text for one GM body. Empty means print nothing.
pub fn render_gm(body: &ServerBody) -> String {
    match body {
        ServerBody::Say(speech) => format!("{}: {}", speaker_name(speech), speech.text),
        ServerBody::Queued { actor, move_id, .. } => {
            format!("{} queued: {move_id}", humanize(actor))
        }
        ServerBody::Status(status) => render_status(status),
        ServerBody::View(view) => render_view(view),
        ServerBody::Committed(body) => render_commit(body),
        _ => String::new(),
    }
}

pub fn render_check(roll: &Roll) -> String {
    let Roll {
        total,
        success_at,
        outcome,
    } = roll;
    format!("Roll: {total} vs {success_at}, {}", outcome.as_str())
}
