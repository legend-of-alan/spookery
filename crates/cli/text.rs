//! Readable text for player frames, GM events, and the status and commit replies.

use crate::protocol::{
    self, exit_phrase, ClockView, CommitBody, ExitView, GmEvent, MeterView, NoteView, Outcome,
    PlayerFrame, Speaker, TableStatus,
};

/// Readable text for one player frame. A new [`PlayerFrame`] variant fails here until it is rendered.
pub fn render(message: &PlayerFrame) -> String {
    match message {
        PlayerFrame::Say {
            from,
            address: _,
            text,
            replay: _,
        } => format!("{}: {text}", speaker(from)),
        PlayerFrame::Queued(_) => "Your action is queued.".to_string(),
        PlayerFrame::Check { outcome } => match outcome {
            Outcome::Success => "Your roll succeeded.".to_string(),
            Outcome::Partial => "Your roll partly succeeded.".to_string(),
            Outcome::Miss => "Your roll missed.".to_string(),
        },
        PlayerFrame::Resume { notes, clocks } => render_resume(notes, clocks),
        PlayerFrame::Lines { text } => text.clone(),
        PlayerFrame::Refused(refusal) => refusal.to_string(),
        PlayerFrame::View(view) => render_view(view),
    }
}

fn speaker(from: &Speaker) -> String {
    match from {
        Speaker::Gm => "GM".to_string(),
        // One player sits this socket, so a player id is the seated PC.
        Speaker::Player(_) => "You".to_string(),
        Speaker::Npc(id) => humanize(id),
    }
}

/// Readable text for the frames of one sitting.
pub struct PlayerFormatter {
    seen_resume: bool,
    earlier_open: bool,
}

impl PlayerFormatter {
    pub fn new() -> Self {
        Self {
            seen_resume: false,
            earlier_open: false,
        }
    }

    /// Text for this frame. Empty means print nothing.
    /// A leading newline is the blank line before the first live frame.
    pub fn format(&mut self, message: &PlayerFrame) -> String {
        match message {
            PlayerFrame::Say {
                from,
                text,
                replay: true,
                ..
            } => self.open_earlier(format!("{}: {text}", speaker(from))),
            PlayerFrame::Resume { notes, clocks } => {
                if self.seen_resume {
                    return String::new();
                }
                self.seen_resume = true;
                self.close_earlier(render_resume(notes, clocks))
            }
            _ => self.close_earlier(render(message)),
        }
    }

    fn open_earlier(&mut self, line: String) -> String {
        if self.earlier_open {
            line
        } else {
            self.earlier_open = true;
            format!("Earlier:\n{line}")
        }
    }

    fn close_earlier(&mut self, text: String) -> String {
        if text.is_empty() || !self.earlier_open {
            return text;
        }
        self.earlier_open = false;
        format!("\n{text}")
    }
}

fn render_resume(notes: &[NoteView], clocks: &[ClockView]) -> String {
    let mut lines = Vec::new();
    for note in notes {
        let text = note.text.trim();
        if !text.is_empty() {
            lines.push(text.to_string());
        }
    }
    for clock in clocks {
        lines.push(clock_line(clock));
    }
    if lines.is_empty() {
        String::new()
    } else {
        format!("While you were away:\n{}", lines.join("\n"))
    }
}

fn render_view(view: &protocol::View) -> String {
    let protocol::View {
        viewer,
        place,
        place_name,
        body,
        actor_names,
        status,
        visible_actors,
        known_facts,
        notes,
        clocks,
        items,
        meters,
        exits,
        unknown_mentions: _,
        habits: _,
    } = view;
    let mut sections = Vec::new();
    let titled = if !place_name.is_empty() {
        place_name.as_str()
    } else {
        place.as_str()
    };
    if !titled.is_empty() {
        sections.push(format!("You are in {titled}."));
    }
    if !status.is_empty() {
        sections.push(status.clone());
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
    if !clocks.is_empty() {
        let mut lines = Vec::new();
        for clock in clocks {
            lines.push(clock_line(clock));
        }
        sections.push(format!("Clocks:\n{}", lines.join("\n")));
    }
    if !items.is_empty() {
        let mut lines = Vec::new();
        for item in items {
            lines.push(format!("{}: {}", humanize(&item.id), item.count));
        }
        sections.push(format!("Items:\n{}", lines.join("\n")));
    }
    if !meters.is_empty() {
        let mut lines = Vec::new();
        for meter in meters {
            lines.push(meter_line(meter));
        }
        sections.push(format!("Meters:\n{}", lines.join("\n")));
    }
    sections.push(exit_section(exits));
    sections.join("\n\n")
}

fn exit_section(exits: &[ExitView]) -> String {
    if exits.is_empty() {
        return "Exits:\nnone".to_string();
    }
    let lines = exits
        .iter()
        .map(|exit| exit_phrase(&exit.name, exit.minutes, exit.locked))
        .collect::<Vec<_>>()
        .join("\n");
    format!("Exits:\n{lines}")
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

fn clock_line(clock: &ClockView) -> String {
    let name = humanize(&clock.id);
    if clock.paused {
        format!("{name}: {}/{}, paused", clock.value, clock.max)
    } else {
        format!("{name}: {}/{}", clock.value, clock.max)
    }
}

/// `docks_shop` becomes "Docks shop". A `player:` or `npc:` prefix is not part of the name.
pub(crate) fn humanize(id: &str) -> String {
    let bare = id
        .strip_prefix("player:")
        .or_else(|| id.strip_prefix("npc:"))
        .unwrap_or(id);
    let spaced = bare.replace('_', " ");
    let mut chars = spaced.chars();
    let Some(first) = chars.next() else {
        return spaced;
    };
    let mut titled = String::new();
    titled.push(first.to_ascii_uppercase());
    titled.push_str(chars.as_str());
    titled
}

fn seated_ids(body: &TableStatus) -> Vec<String> {
    body.presence.actors_present.clone()
}

fn queue_rows(body: &TableStatus) -> Vec<(String, String)> {
    body.queued
        .iter()
        .map(|row| (row.actor_id.clone(), row.move_name.clone()))
        .collect()
}

pub fn render_status(body: &TableStatus) -> String {
    let names = seated_ids(body);
    let seated = if names.is_empty() {
        "Seated: nobody".to_string()
    } else {
        let names = names
            .iter()
            .map(|id| actor_name(id))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Seated: {names}")
    };
    let queue = queue_rows(body);
    let queued = if queue.is_empty() {
        "Queue: empty".to_string()
    } else {
        let lines = queue
            .iter()
            .map(|(id, move_name)| format!("{}: {move_name}", actor_name(id)))
            .collect::<Vec<_>>()
            .join("\n");
        format!("Queue:\n{lines}")
    };
    format!("{seated}\n{queued}")
}

pub fn render_diff(body: &CommitBody) -> String {
    if body.changed_keys.is_empty() {
        "Nothing changed.".to_string()
    } else {
        format!("Changed:\n{}", body.changed_keys.join("\n"))
    }
}

pub fn render_dice(body: &CommitBody) -> Vec<String> {
    body.dice
        .iter()
        .map(|line| format!("dice {line}"))
        .collect()
}

pub fn render_checks(body: &CommitBody) -> Vec<String> {
    body.rolls
        .iter()
        .map(|roll| {
            render_check(
                &roll.faces,
                roll.stat,
                roll.total,
                roll.target,
                roll.outcome.as_str(),
            )
        })
        .collect()
}

pub fn actor_name(id: &str) -> String {
    if id == "gm" {
        "GM".to_string()
    } else {
        humanize(id)
    }
}

pub fn render_event(body: &GmEvent) -> String {
    match body {
        GmEvent::Say(say) => format!("{}: {}", actor_name(&say.from.to_string()), say.text),
        GmEvent::Queued(row) => {
            format!("{} queued: {}", actor_name(&row.actor_id), row.move_name)
        }
        GmEvent::Check(roll) => render_check(
            &roll.faces,
            roll.stat,
            roll.total,
            roll.target,
            roll.outcome.as_str(),
        ),
    }
}

pub fn render_check(faces: &[u32], stat: i64, total: i64, target: i64, outcome: &str) -> String {
    let dice = faces
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(" + ");
    let sign = if stat >= 0 {
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
    use crate::protocol::{
        ItemStack, KnownFact, NoteView, Queued, QueuedMove, Roll, SayEvent, StatusPause,
        StatusPresence,
    };

    fn assert_readable(text: &str) {
        assert!(!text.contains('{'), "{text}");
        assert!(!text.contains("\"kind\""), "{text}");
    }

    #[test]
    fn a_player_prefix_is_not_part_of_the_name() {
        assert_eq!(humanize("player:pc_rowan"), "Pc rowan");
        assert_eq!(humanize("npc:mira"), "Mira");
        assert_eq!(humanize("docks_shop"), "Docks shop");
        assert_eq!(humanize("pc_rowan"), "Pc rowan");
        assert_eq!(humanize("mira"), "Mira");
    }

    #[test]
    fn a_say_is_a_name_and_the_text() {
        let mira = render(&PlayerFrame::Say {
            from: Speaker::Npc("mira".to_string()),
            address: "table".to_string(),
            text: "the lamp is lit".to_string(),
            replay: false,
        });
        assert_eq!(mira, "Mira: the lamp is lit");
        assert_readable(&mira);

        let gm = render(&PlayerFrame::Say {
            from: Speaker::Gm,
            address: "table".to_string(),
            text: "evening".to_string(),
            replay: false,
        });
        assert_eq!(gm, "GM: evening");
        assert_readable(&gm);

        let you = render(&PlayerFrame::Say {
            from: Speaker::Player("pc_rowan".to_string()),
            address: "actor".to_string(),
            text: "hello".to_string(),
            replay: false,
        });
        assert_eq!(you, "You: hello");
        assert_readable(&you);
    }

    #[test]
    fn a_queued_action_is_a_sentence() {
        let text = render(&PlayerFrame::Queued(Queued {
            id: "queue-id-42".to_string(),
        }));
        assert_eq!(text, "Your action is queued.");
        assert!(!text.contains("queue-id-42"), "{text}");
        assert_readable(&text);
    }

    #[test]
    fn a_check_is_a_short_sentence() {
        let yes = render(&PlayerFrame::Check {
            outcome: crate::protocol::Outcome::Success,
        });
        let partial = render(&PlayerFrame::Check {
            outcome: crate::protocol::Outcome::Partial,
        });
        let miss = render(&PlayerFrame::Check {
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
        let text = render(&PlayerFrame::Resume {
            notes: vec![note("note_shop", "the shop restocked")],
            clocks: Vec::new(),
        });
        assert_eq!(text, "While you were away:\nthe shop restocked");
        assert_readable(&text);
        assert_eq!(
            render(&PlayerFrame::Resume {
                notes: Vec::new(),
                clocks: Vec::new(),
            }),
            ""
        );
    }

    #[test]
    fn a_later_resume_has_no_away_heading() {
        let mut formatter = PlayerFormatter::new();
        let first = formatter.format(&PlayerFrame::Resume {
            notes: vec![note("note_shop", "the shop restocked")],
            clocks: Vec::new(),
        });
        assert_eq!(first, "While you were away:\nthe shop restocked");
        assert_eq!(
            formatter.format(&PlayerFrame::Resume {
                notes: Vec::new(),
                clocks: Vec::new(),
            }),
            ""
        );
        let later = formatter.format(&PlayerFrame::Resume {
            notes: vec![note("note_yard", "the yard stays watched")],
            clocks: Vec::new(),
        });
        assert_eq!(later, "");
    }

    #[test]
    fn replayed_speech_sits_under_one_heading() {
        let mut formatter = PlayerFormatter::new();
        let first = formatter.format(&PlayerFrame::Say {
            from: Speaker::Player("pc_lark".to_string()),
            address: "table".to_string(),
            text: "hello Briar".to_string(),
            replay: true,
        });
        assert_eq!(first, "Earlier:\nYou: hello Briar");
        let second = formatter.format(&PlayerFrame::Say {
            from: Speaker::Gm,
            address: "table".to_string(),
            text: "evening".to_string(),
            replay: true,
        });
        assert_eq!(second, "GM: evening");
        assert_eq!(
            formatter.format(&PlayerFrame::Resume {
                notes: Vec::new(),
                clocks: Vec::new(),
            }),
            ""
        );
        let place = formatter.format(&PlayerFrame::Lines {
            text: "You are in Gate Yard.".to_string(),
        });
        assert_eq!(place, "\nYou are in Gate Yard.");
    }

    #[test]
    fn a_resume_turns_digest_rows_into_sentences() {
        let text = render(&PlayerFrame::Resume {
            notes: vec![note("note_mira_door", "Back at dusk. Oil on the shelf.")],
            clocks: vec![
                ClockView {
                    id: "assassination_debt".to_string(),
                    value: 2,
                    max: 6,
                    paused: true,
                },
                ClockView {
                    id: "shop_day".to_string(),
                    value: 1,
                    max: 1,
                    paused: false,
                },
            ],
        });
        assert_eq!(
            text,
            "\
While you were away:
Back at dusk. Oil on the shelf.
Assassination debt: 2/6, paused
Shop day: 1/1"
        );
        assert!(!text.contains("note_mira_door"), "{text}");
        assert!(!text.contains("assassination_debt"), "{text}");
        assert!(!text.contains("running"), "{text}");
        assert_readable(&text);
    }

    #[test]
    fn a_perceive_names_the_place_and_skips_empty_sections() {
        let text = render(&PlayerFrame::View(Box::new(protocol::View {
            viewer: "pc_rowan".to_string(),
            place: "docks_shop".to_string(),
            place_name: "Docks Shop".to_string(),
            body: "The shop faces the water. Tins sit in a row.".to_string(),
            actor_names: vec!["Aldric".to_string(), "Rowan".to_string()],
            status: String::new(),
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
            clocks: vec![
                ClockView {
                    id: "assassination_debt".to_string(),
                    value: 2,
                    max: 6,
                    paused: true,
                },
                ClockView {
                    id: "shop_day".to_string(),
                    value: 1,
                    max: 1,
                    paused: false,
                },
            ],
            items: vec![ItemStack {
                id: "lamp_oil".to_string(),
                count: 4,
            }],
            meters: vec![MeterView {
                id: "grit".to_string(),
                value: 1,
                max: 110,
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

Clocks:
Assassination debt: 2/6, paused
Shop day: 1/1

Items:
Lamp oil: 4

Meters:
Grit: 1/110

Exits:
stairs (locked) — 2 min"
        );
        assert!(!text.contains("CANON-BODY"), "{text}");
        assert!(!text.contains("secret_fact"), "{text}");
        assert!(!text.contains("tend_shop"), "{text}");
        assert!(!text.contains("note_door"), "{text}");
        assert!(!text.contains("docks_shop"), "{text}");
        assert_readable(&text);

        let quiet = render(&PlayerFrame::View(Box::new(protocol::View {
            viewer: "pc_rowan".to_string(),
            place: "docks_street".to_string(),
            place_name: "Docks Street".to_string(),
            body: String::new(),
            actor_names: vec!["Bram".to_string()],
            status: "on the stairs until 21:40".to_string(),
            visible_actors: vec!["bram".to_string()],
            known_facts: Vec::new(),
            notes: Vec::new(),
            unknown_mentions: Vec::new(),
            clocks: Vec::new(),
            items: Vec::new(),
            meters: Vec::new(),
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
        assert!(!quiet.contains("Clocks:"), "{quiet}");
        assert!(!quiet.contains("Items:"), "{quiet}");
        assert!(!quiet.contains("Meters:"), "{quiet}");
        assert_readable(&quiet);
    }

    fn say(from: &str, seq: u64, text: &str) -> GmEvent {
        GmEvent::Say(SayEvent {
            seq,
            from: from.parse().unwrap(),
            address: "table".to_string(),
            place: None,
            actor: None,
            recipients: Vec::new(),
            text: text.to_string(),
        })
    }

    fn roll(faces: Vec<u32>, stat: i64, total: i64, target: i64, outcome: Outcome) -> GmEvent {
        GmEvent::Check(Roll {
            faces,
            stat,
            total,
            target,
            outcome,
        })
    }

    fn status(present: Vec<&str>, queued: Vec<QueuedMove>) -> TableStatus {
        TableStatus {
            presence: StatusPresence {
                players: u32::try_from(present.len()).unwrap_or(0),
                actors_present: present.into_iter().map(str::to_string).collect(),
            },
            pause: StatusPause {
                presence_clocks: "running".to_string(),
            },
            world_time: 0,
            round_index: 0,
            state_rev: 0,
            pack_rev: String::new(),
            last_round_at: String::new(),
            queued,
            last_seq: 0,
            events: None,
            pack: None,
        }
    }

    #[test]
    fn each_event_renders_as_text() {
        let player = render_event(&say("player:pc_rowan", 1, "hello"));
        assert_eq!(player, "Pc rowan: hello");
        assert!(!player.contains('{'));

        let gm = render_event(&say("gm", 2, "good evening"));
        assert_eq!(gm, "GM: good evening");

        let npc = render_event(&say("npc:mira", 3, "oil"));
        assert_eq!(npc, "Mira: oil");

        let queued = render_event(&GmEvent::Queued(QueuedMove {
            id: "id-1".to_string(),
            actor_id: "pc_wren".to_string(),
            move_name: "wait".to_string(),
        }));
        assert_eq!(queued, "Pc wren queued: wait");

        let success = render_event(&roll(vec![4, 2], 1, 7, 0, Outcome::Success));
        assert_eq!(success, "Roll: 4 + 2, +1 = 7 vs 0, success");

        let partial = render_event(&roll(vec![3, 3], 1, 7, 10, Outcome::Partial));
        assert_eq!(partial, "Roll: 3 + 3, +1 = 7 vs 10, partial");

        let failure = render_event(&roll(vec![1], -2, -1, 3, Outcome::Miss));
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
}
