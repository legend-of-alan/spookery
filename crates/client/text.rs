//! Readable text for player and GM messages, and the status and commit replies.

use crate::protocol::{
    self, CommitBody, ExitView, MeterView, NoteView, Outcome, PlaceView, Roll, Say, ServerBody,
    Speaker, TableStatus, WordView,
};

/// Readable text for one player body. Empty means print nothing.
pub fn render(body: &ServerBody) -> String {
    match body {
        ServerBody::Say(say) => render_say(say),
        ServerBody::Queued { .. } => "Your action is queued.".to_string(),
        ServerBody::Check { outcome } => match outcome {
            Outcome::Success => "Your roll succeeded.".to_string(),
            Outcome::Partial => "Your roll partly succeeded.".to_string(),
            Outcome::Miss => "Your roll missed.".to_string(),
        },
        ServerBody::Resume { notes } => render_resume(notes),
        ServerBody::Lines { text } => text.clone(),
        ServerBody::Refused(refusal) => refusal.to_string(),
        ServerBody::View(view) => render_view(view),
        ServerBody::Help { words } => render_help(words),
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

fn render_say(say: &Say) -> String {
    match &say.from {
        // One player sits this socket, so a player id is the seated PC.
        Speaker::Player(_) => format!("You: {}", say.text),
        from @ (Speaker::Gm | Speaker::Npc(_)) => format!("{}: {}", speaker_name(from), say.text),
    }
}

/// The says a claim replays, under an `Earlier:` heading.
fn render_scrollback(says: &[Say]) -> String {
    if says.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = says.iter().map(render_say).collect();
    format!(
        "Earlier:
{}",
        lines.join(
            "
"
        )
    )
}

/// Readable text for the messages of one sitting.
pub struct PlayerText {
    seen_resume: bool,
    after_scrollback: bool,
}

impl PlayerText {
    pub fn new() -> Self {
        Self {
            seen_resume: false,
            after_scrollback: false,
        }
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

/// The place name a view shows, or its id when it has none.
fn place_title(view: &protocol::View) -> &str {
    if view.place_name.is_empty() {
        &view.place
    } else {
        &view.place_name
    }
}

pub fn render_view(view: &protocol::View) -> String {
    let protocol::View {
        viewer,
        place: _,
        place_name: _,
        body,
        actor_names,
        cant_act,
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
    let titled = place_title(view);
    if !titled.is_empty() {
        sections.push(format!("You are in {titled}."));
    }
    if !cant_act.is_empty() {
        sections.push(cant_act.clone());
    }
    let prose = body.trim();
    if !prose.is_empty() {
        sections.push(prose.to_string());
    }
    if !visible_actors.is_empty() {
        let names = visible_actors
            .iter()
            .enumerate()
            .map(|(index, id)| {
                if id == viewer {
                    "You".to_string()
                } else {
                    actor_names
                        .get(index)
                        .filter(|name| !name.is_empty())
                        .cloned()
                        .unwrap_or_else(|| id.to_string())
                }
            })
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
        let mut lines = Vec::new();
        for item in items {
            lines.push(format!("{}: {}", humanize(&item.id), item.count));
        }
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

/// One word per line: `look`, `ask briar`, or `strike <actor>: briar, aldric`.
pub fn render_help(words: &[WordView]) -> String {
    let lines: Vec<String> = words.iter().map(word_line).collect();
    lines.join("\n")
}

fn word_line(word: &WordView) -> String {
    let name = &word.name;
    if let Some(owner) = &word.owner {
        return format!("{name} {owner}");
    }
    if word.target == "none" {
        return name.clone();
    }
    let line = format!("{name} <{}>", word.target);
    if word.names.is_empty() {
        line
    } else {
        format!("{line}: {}", word.names.join(", "))
    }
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

/// `stairs (locked) — 2 min`.
fn exit_phrase(exit: &ExitView) -> String {
    let (name, minutes) = (&exit.name, exit.minutes);
    if exit.locked {
        format!("{name} (locked) — {minutes} min")
    } else {
        format!("{name} — {minutes} min")
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

fn meter_line(meter: &MeterView) -> String {
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
            .map(|row| format!("{}: {}", humanize(&row.actor_id), row.move_name))
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

/// The name a say shows for who spoke: `GM`, or the speaker's id as words.
fn speaker_name(from: &Speaker) -> String {
    match from {
        Speaker::Gm => "GM".to_string(),
        Speaker::Player(id) | Speaker::Npc(id) => humanize(id),
    }
}

/// Readable text for one GM body. Empty means print nothing.
pub fn render_gm(body: &ServerBody) -> String {
    match body {
        ServerBody::Say(say) => format!("{}: {}", speaker_name(&say.from), say.text),
        ServerBody::Queued {
            actor, move_name, ..
        } => format!("{} queued: {move_name}", humanize(actor)),
        ServerBody::Status(status) => render_status(status),
        ServerBody::View(view) => render_view(view),
        ServerBody::Committed(body) => render_commit(body),
        _ => String::new(),
    }
}

pub fn render_check(roll: &Roll) -> String {
    let Roll {
        faces,
        stat,
        total,
        target,
        outcome,
    } = roll;
    let outcome = outcome.as_str();
    let dice = faces
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" + ");
    let sign = if *stat >= 0 {
        format!("+{stat}")
    } else {
        stat.to_string()
    };
    if dice.is_empty() {
        format!("Roll: {sign} = {total} vs {target}, {outcome}")
    } else {
        format!("Roll: {dice}, {sign} = {total} vs {target}, {outcome}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ItemStack, KnownFact, NoteView, QueuedMove, Roll, StatusPresence};

    fn assert_readable(text: &str) {
        assert!(!text.contains('{'), "{text}");
        assert!(!text.contains("\"kind\""), "{text}");
    }

    #[test]
    fn an_id_reads_as_words() {
        assert_eq!(humanize("docks_shop"), "Docks shop");
        assert_eq!(humanize("pc_rowan"), "Pc rowan");
        assert_eq!(humanize("mira"), "Mira");
    }

    #[test]
    fn a_say_is_a_name_and_the_text() {
        let mira = render(&ServerBody::Say(Say {
            from: Speaker::Npc("mira".to_string()),
            address: "table".to_string(),
            text: "the lamp is lit".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
        }));
        assert_eq!(mira, "Mira: the lamp is lit");
        assert_readable(&mira);

        let gm = render(&ServerBody::Say(Say {
            from: Speaker::Gm,
            address: "table".to_string(),
            text: "evening".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
        }));
        assert_eq!(gm, "GM: evening");
        assert_readable(&gm);

        let you = render(&ServerBody::Say(Say {
            from: Speaker::Player("pc_rowan".to_string()),
            address: "actor".to_string(),
            text: "hello".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
        }));
        assert_eq!(you, "You: hello");
        assert_readable(&you);
    }

    #[test]
    fn a_queued_action_is_a_sentence() {
        let text = render(&ServerBody::Queued {
            intent: "queue-id-42".to_string(),
            actor: "pc_rowan".to_string(),
            move_name: "wait".to_string(),
            target: None,
        });
        assert_eq!(text, "Your action is queued.");
        assert!(!text.contains("queue-id-42"), "{text}");
        assert_readable(&text);
    }

    #[test]
    fn a_check_is_a_short_sentence() {
        let yes = render(&ServerBody::Check {
            outcome: crate::protocol::Outcome::Success,
        });
        let partial = render(&ServerBody::Check {
            outcome: crate::protocol::Outcome::Partial,
        });
        let miss = render(&ServerBody::Check {
            outcome: crate::protocol::Outcome::Miss,
        });
        assert_eq!(yes, "Your roll succeeded.");
        assert_eq!(partial, "Your roll partly succeeded.");
        assert_eq!(miss, "Your roll missed.");
        assert_readable(&yes);
        assert_readable(&partial);
        assert_readable(&miss);
    }

    fn note(id: &str, text: &str) -> NoteView {
        NoteView {
            id: id.to_string(),
            to: vec!["pc_rowan".to_string()],
            place: Some("docks_shop".to_string()),
            fact_ids: Vec::new(),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_resume_is_an_away_block() {
        let text = render(&ServerBody::Resume {
            notes: vec![note("note_shop", "the shop restocked")],
        });
        assert_eq!(text, "While you were away:\nthe shop restocked");
        assert_readable(&text);
        assert_eq!(render(&ServerBody::Resume { notes: Vec::new() }), "");
    }

    #[test]
    fn a_later_resume_has_no_away_heading() {
        let mut player_text = PlayerText::new();
        let first = player_text.render(&ServerBody::Resume {
            notes: vec![note("note_shop", "the shop restocked")],
        });
        assert_eq!(first, "While you were away:\nthe shop restocked");
        assert_eq!(
            player_text.render(&ServerBody::Resume { notes: Vec::new() }),
            ""
        );
        let later = player_text.render(&ServerBody::Resume {
            notes: vec![note("note_yard", "the yard stays watched")],
        });
        assert_eq!(later, "");
    }

    fn table_say(from: Speaker, text: &str) -> Say {
        Say {
            from,
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
            text: text.to_string(),
        }
    }

    #[test]
    fn replayed_speech_sits_under_one_heading() {
        let mut player_text = PlayerText::new();
        let claimed = player_text.render(&ServerBody::Claimed {
            actor: "pc_lark".to_string(),
            token: "t1".to_string(),
            scrollback: vec![
                table_say(Speaker::Player("pc_lark".to_string()), "hello Briar"),
                table_say(Speaker::Gm, "evening"),
            ],
            missed: Vec::new(),
            gap: false,
        });
        assert_eq!(
            claimed,
            "Earlier:
You: hello Briar
GM: evening"
        );
        assert_eq!(
            player_text.render(&ServerBody::Resume { notes: Vec::new() }),
            ""
        );
        let place = player_text.render(&ServerBody::Lines {
            text: "You are in Gate Yard.".to_string(),
        });
        assert_eq!(
            place,
            "
You are in Gate Yard."
        );
        let quiet = PlayerText::new().render(&ServerBody::Claimed {
            actor: "pc_lark".to_string(),
            token: "t1".to_string(),
            scrollback: Vec::new(),
            missed: Vec::new(),
            gap: false,
        });
        assert_eq!(quiet, "");
    }

    #[test]
    fn a_perceive_names_the_place_and_skips_empty_sections() {
        let text = render(&ServerBody::View(Box::new(protocol::View {
            viewer: "pc_rowan".to_string(),
            place: "docks_shop".to_string(),
            place_name: "Docks Shop".to_string(),
            body: "The shop faces the water. Tins sit in a row.".to_string(),
            actor_names: vec!["Aldric".to_string(), "Rowan".to_string()],
            cant_act: String::new(),
            visible_actors: vec!["aldric".to_string(), "pc_rowan".to_string()],
            known_facts: vec![KnownFact {
                id: "fact_oil_on_shelf".to_string(),
                text: "Oil is on the shelf.".to_string(),
                owner_body: "CANON-BODY".to_string(),
            }],
            notes: vec![NoteView {
                id: "note_door".to_string(),
                to: vec!["pc_rowan".to_string()],
                place: Some("docks_shop".to_string()),
                fact_ids: vec!["fact_oil_on_shelf".to_string()],
                text: "Back in an hour.".to_string(),
            }],
            unknown_mentions: vec!["secret_fact".to_string()],
            items: vec![ItemStack {
                id: "lamp_oil".to_string(),
                count: 4,
            }],
            meters: vec![MeterView {
                id: "grit".to_string(),
                value: 1,
                max: 110,
            }],
            world_meters: vec![MeterView {
                id: "dawn".to_string(),
                value: 3,
                max: 6,
            }],
            habits: vec!["tend_shop".to_string()],
            exits: vec![crate::protocol::ExitView {
                name: "stairs".to_string(),
                to: "mira_rooms".to_string(),
                place_name: "Mira's Rooms".to_string(),
                minutes: 2,
                locked: true,
            }],
        })));
        assert_eq!(
            text,
            "\
You are in Docks Shop.

The shop faces the water. Tins sit in a row.

Here: Aldric, You

You know:
Oil is on the shelf.

Notes:
Back in an hour.

Items:
Lamp oil: 4

Meters:
Grit: 1/110

The world:
Dawn: 3/6

Exits:
stairs (locked) — 2 min"
        );
        assert!(!text.contains("CANON-BODY"), "{text}");
        assert!(!text.contains("secret_fact"), "{text}");
        assert!(!text.contains("tend_shop"), "{text}");
        assert!(!text.contains("note_door"), "{text}");
        assert!(!text.contains("docks_shop"), "{text}");
        assert_readable(&text);

        let quiet = render(&ServerBody::View(Box::new(protocol::View {
            viewer: "pc_rowan".to_string(),
            place: "docks_street".to_string(),
            place_name: "Docks Street".to_string(),
            body: String::new(),
            actor_names: vec!["Bram".to_string()],
            cant_act: "on the stairs until 21:40".to_string(),
            visible_actors: vec!["bram".to_string()],
            known_facts: Vec::new(),
            notes: Vec::new(),
            unknown_mentions: Vec::new(),
            items: Vec::new(),
            meters: Vec::new(),
            world_meters: Vec::new(),
            habits: Vec::new(),
            exits: Vec::new(),
        })));
        assert_eq!(
            quiet,
            "\
You are in Docks Street.

on the stairs until 21:40

Here: Bram

Exits:
none"
        );
        assert!(!quiet.contains("You know:"), "{quiet}");
        assert!(!quiet.contains("Notes:"), "{quiet}");
        assert!(!quiet.contains("The world:"), "{quiet}");
        assert!(!quiet.contains("Items:"), "{quiet}");
        assert!(!quiet.contains("Meters:"), "{quiet}");
        assert_readable(&quiet);
    }

    fn say(from: &str, text: &str) -> ServerBody {
        ServerBody::Say(table_say(from.parse().unwrap(), text))
    }

    fn roll(faces: Vec<u32>, stat: i64, total: i64, target: i64, outcome: Outcome) -> Roll {
        Roll {
            faces,
            stat,
            total,
            target,
            outcome,
        }
    }

    fn status(present: Vec<&str>, queued: Vec<QueuedMove>) -> TableStatus {
        TableStatus {
            presence: StatusPresence {
                players: u32::try_from(present.len()).unwrap_or(0),
                actors_present: present.into_iter().map(str::to_string).collect(),
            },
            world_time: 0,
            round_index: 0,
            state_rev: 0,
            pack_rev: String::new(),
            last_round_at: String::new(),
            queued,
            pack: None,
        }
    }

    #[test]
    fn each_gm_body_renders_as_text() {
        let player = render_gm(&say("player:pc_rowan", "hello"));
        assert_eq!(player, "Pc rowan: hello");
        assert!(!player.contains('{'));

        let gm = render_gm(&say("gm", "good evening"));
        assert_eq!(gm, "GM: good evening");

        let npc = render_gm(&say("npc:mira", "oil"));
        assert_eq!(npc, "Mira: oil");

        let queued = render_gm(&ServerBody::Queued {
            intent: "id-1".to_string(),
            actor: "pc_wren".to_string(),
            move_name: "wait".to_string(),
            target: None,
        });
        assert_eq!(queued, "Pc wren queued: wait");
        assert_eq!(
            render_gm(&ServerBody::Round {
                round_index: 1,
                world_time: 10
            }),
            ""
        );

        let success = render_check(&roll(vec![4, 2], 1, 7, 0, Outcome::Success));
        assert_eq!(success, "Roll: 4 + 2, +1 = 7 vs 0, success");

        let partial = render_check(&roll(vec![3, 3], 1, 7, 10, Outcome::Partial));
        assert_eq!(partial, "Roll: 3 + 3, +1 = 7 vs 10, partial");

        let failure = render_check(&roll(vec![1], -2, -1, 3, Outcome::Miss));
        assert_eq!(failure, "Roll: 1, -2 = -1 vs 3, miss");
        assert!(!failure.contains('{'));
    }

    #[test]
    fn a_status_names_the_seated_players_and_the_queue() {
        let body = status(
            vec!["pc_rowan"],
            vec![QueuedMove {
                id: "q".to_string(),
                actor_id: "pc_rowan".to_string(),
                move_name: "wait".to_string(),
            }],
        );
        assert_eq!(
            render_status(&body),
            "Seated: Pc rowan\nQueue:\nPc rowan: wait"
        );
        assert_eq!(
            render_status(&status(Vec::new(), Vec::new())),
            "Seated: nobody\nQueue: empty"
        );
    }

    #[test]
    fn a_commit_shows_its_changes_rolls_and_dice() {
        let body = CommitBody {
            state_rev: 2,
            changed_keys: vec!["actors.pc_rowan.meters.grit".to_string()],
            journal_path: None,
            rolls: vec![roll(vec![4, 2], 1, 7, 0, Outcome::Success)],
            dice: vec!["2d6 = 3 + 4 = 7".to_string()],
        };
        assert_eq!(
            render_gm(&ServerBody::Committed(body)),
            "Changed:\nactors.pc_rowan.meters.grit\nRoll: 4 + 2, +1 = 7 vs 0, success\ndice 2d6 = 3 + 4 = 7"
        );
    }
}
