//! Readable text for one player frame.

use crate::protocol::{
    exit_phrase, named_refusal, Outcome, PlayerMessage, WireClock, WireExit, WireMessage, WireMeter,
};

/// Readable text for one player frame. A new [`PlayerMessage`] variant fails here until it is rendered.
pub fn render(message: &PlayerMessage) -> String {
    match message {
        PlayerMessage::Say {
            from,
            address: _,
            text,
            replay: _,
        } => format!("{}: {text}", speaker(from)),
        PlayerMessage::Queued { id: _ } => "Your action is queued.".to_string(),
        PlayerMessage::Check { outcome } => match outcome {
            Outcome::Success => "Your roll succeeded.".to_string(),
            Outcome::Partial => "Your roll partly succeeded.".to_string(),
            Outcome::Miss => "Your roll missed.".to_string(),
        },
        PlayerMessage::Resume { messages, clocks } => resume(messages, clocks),
        PlayerMessage::Lines { text } => text.clone(),
        PlayerMessage::Refused { code, missing } => named_refusal(code, missing.as_ref()),
        PlayerMessage::View { .. } => view_text(message),
    }
}

fn speaker(from: &str) -> String {
    if from == "gm" {
        return "GM".to_string();
    }
    // One player sits this socket, so a player id is the seated PC.
    if from
        .strip_prefix("player:")
        .is_some_and(|id| !id.is_empty())
    {
        return "You".to_string();
    }
    humanize(from)
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
    pub fn show(&mut self, message: &PlayerMessage) -> String {
        match message {
            PlayerMessage::Say {
                from,
                text,
                replay: true,
                ..
            } => self.open_earlier(format!("{}: {text}", speaker(from))),
            PlayerMessage::Resume { messages, clocks } => {
                if self.seen_resume {
                    return String::new();
                }
                self.seen_resume = true;
                self.close_earlier(resume(messages, clocks))
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

fn resume(messages: &[WireMessage], clocks: &[WireClock]) -> String {
    let mut lines = Vec::new();
    for message in messages {
        let text = message.text.trim();
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

fn view_text(message: &PlayerMessage) -> String {
    let PlayerMessage::View {
        viewer,
        place,
        scene,
        visible_actors,
        known_facts,
        messages,
        clocks,
        items,
        meters,
        exits,
        unknown_mentioned: _,
        habits: _,
    } = message
    else {
        return String::new();
    };
    let mut sections = Vec::new();
    let titled = if !scene.place_name.is_empty() {
        scene.place_name.as_str()
    } else {
        place.as_str()
    };
    if !titled.is_empty() {
        sections.push(format!("You are in {titled}."));
    }
    if !scene.status.is_empty() {
        sections.push(scene.status.clone());
    }
    let prose = scene.prose.trim();
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
                    scene
                        .actor_names
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
        messages.iter().map(|message| message.text.as_str()),
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

fn exit_section(exits: &[WireExit]) -> String {
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

fn meter_line(meter: &WireMeter) -> String {
    format!("{}: {}/{}", humanize(&meter.id), meter.value, meter.max)
}

fn clock_line(clock: &WireClock) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{WireFact, WireItem, WireMessage};

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
        let mira = render(&PlayerMessage::Say {
            from: "npc:mira".to_string(),
            address: "table".to_string(),
            text: "the lamp is lit".to_string(),
            replay: false,
        });
        assert_eq!(mira, "Mira: the lamp is lit");
        assert_readable(&mira);

        let gm = render(&PlayerMessage::Say {
            from: "gm".to_string(),
            address: "table".to_string(),
            text: "evening".to_string(),
            replay: false,
        });
        assert_eq!(gm, "GM: evening");
        assert_readable(&gm);

        let you = render(&PlayerMessage::Say {
            from: "player:pc_rowan".to_string(),
            address: "actor".to_string(),
            text: "hello".to_string(),
            replay: false,
        });
        assert_eq!(you, "You: hello");
        assert_readable(&you);
    }

    #[test]
    fn a_queued_action_is_a_sentence() {
        let text = render(&PlayerMessage::Queued {
            id: "queue-id-42".to_string(),
        });
        assert_eq!(text, "Your action is queued.");
        assert!(!text.contains("queue-id-42"), "{text}");
        assert_readable(&text);
    }

    #[test]
    fn a_check_is_a_short_sentence() {
        let yes = render(&PlayerMessage::Check {
            outcome: crate::protocol::Outcome::Success,
        });
        let partial = render(&PlayerMessage::Check {
            outcome: crate::protocol::Outcome::Partial,
        });
        let miss = render(&PlayerMessage::Check {
            outcome: crate::protocol::Outcome::Miss,
        });
        assert_eq!(yes, "Your roll succeeded.");
        assert_eq!(partial, "Your roll partly succeeded.");
        assert_eq!(miss, "Your roll missed.");
        assert_readable(&yes);
        assert_readable(&partial);
        assert_readable(&miss);
    }

    fn note(id: &str, text: &str) -> WireMessage {
        WireMessage {
            id: id.to_string(),
            to: vec!["pc_rowan".to_string()],
            place: Some("docks_shop".to_string()),
            fact_ids: Vec::new(),
            text: text.to_string(),
        }
    }

    #[test]
    fn a_resume_is_an_away_block() {
        let text = render(&PlayerMessage::Resume {
            messages: vec![note("note_shop", "the shop restocked")],
            clocks: Vec::new(),
        });
        assert_eq!(text, "While you were away:\nthe shop restocked");
        assert_readable(&text);
        assert_eq!(
            render(&PlayerMessage::Resume {
                messages: Vec::new(),
                clocks: Vec::new(),
            }),
            ""
        );
    }

    #[test]
    fn a_later_resume_has_no_away_heading() {
        let mut formatter = PlayerFormatter::new();
        let first = formatter.show(&PlayerMessage::Resume {
            messages: vec![note("note_shop", "the shop restocked")],
            clocks: Vec::new(),
        });
        assert_eq!(first, "While you were away:\nthe shop restocked");
        assert_eq!(
            formatter.show(&PlayerMessage::Resume {
                messages: Vec::new(),
                clocks: Vec::new(),
            }),
            ""
        );
        let later = formatter.show(&PlayerMessage::Resume {
            messages: vec![note("note_yard", "the yard stays watched")],
            clocks: Vec::new(),
        });
        assert_eq!(later, "");
    }

    #[test]
    fn replayed_speech_sits_under_one_heading() {
        let mut formatter = PlayerFormatter::new();
        let first = formatter.show(&PlayerMessage::Say {
            from: "player:pc_lark".to_string(),
            address: "table".to_string(),
            text: "hello Briar".to_string(),
            replay: true,
        });
        assert_eq!(first, "Earlier:\nYou: hello Briar");
        let second = formatter.show(&PlayerMessage::Say {
            from: "gm".to_string(),
            address: "table".to_string(),
            text: "evening".to_string(),
            replay: true,
        });
        assert_eq!(second, "GM: evening");
        assert_eq!(
            formatter.show(&PlayerMessage::Resume {
                messages: Vec::new(),
                clocks: Vec::new(),
            }),
            ""
        );
        let place = formatter.show(&PlayerMessage::Lines {
            text: "You are in Gate Yard.".to_string(),
        });
        assert_eq!(place, "\nYou are in Gate Yard.");
    }

    #[test]
    fn a_resume_turns_digest_rows_into_sentences() {
        let text = render(&PlayerMessage::Resume {
            messages: vec![note("note_mira_door", "Back at dusk. Oil on the shelf.")],
            clocks: vec![
                WireClock {
                    id: "assassination_debt".to_string(),
                    value: 2,
                    max: 6,
                    paused: true,
                },
                WireClock {
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
        let text = render(&PlayerMessage::View {
            viewer: "pc_rowan".to_string(),
            place: "docks_shop".to_string(),
            scene: Box::new(crate::protocol::ViewScene {
                place_name: "Docks Shop".to_string(),
                prose: "The shop faces the water. Tins sit in a row.".to_string(),
                actor_names: vec!["Aldric".to_string(), "Rowan".to_string()],
                status: String::new(),
            }),
            visible_actors: vec!["aldric".to_string(), "pc_rowan".to_string()],
            known_facts: vec![WireFact {
                id: "fact_oil_on_shelf".to_string(),
                text: "Oil is on the shelf.".to_string(),
                body: "CANON-BODY".to_string(),
            }],
            messages: vec![WireMessage {
                id: "note_door".to_string(),
                to: vec!["pc_rowan".to_string()],
                place: Some("docks_shop".to_string()),
                fact_ids: vec!["fact_oil_on_shelf".to_string()],
                text: "Back in an hour.".to_string(),
            }],
            unknown_mentioned: vec!["secret_fact".to_string()],
            clocks: vec![
                WireClock {
                    id: "assassination_debt".to_string(),
                    value: 2,
                    max: 6,
                    paused: true,
                },
                WireClock {
                    id: "shop_day".to_string(),
                    value: 1,
                    max: 1,
                    paused: false,
                },
            ],
            items: vec![WireItem {
                id: "lamp_oil".to_string(),
                count: 4,
            }],
            meters: vec![WireMeter {
                id: "grit".to_string(),
                value: 1,
                max: 110,
            }],
            habits: vec!["tend_shop".to_string()],
            exits: vec![crate::protocol::WireExit {
                name: "stairs".to_string(),
                to: "mira_rooms".to_string(),
                place_name: "Mira's Rooms".to_string(),
                minutes: 2,
                locked: true,
            }],
        });
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

        let quiet = render(&PlayerMessage::View {
            viewer: "pc_rowan".to_string(),
            place: "docks_street".to_string(),
            scene: Box::new(crate::protocol::ViewScene {
                place_name: "Docks Street".to_string(),
                prose: String::new(),
                actor_names: vec!["Bram".to_string()],
                status: "on the stairs until 21:40".to_string(),
            }),
            visible_actors: vec!["bram".to_string()],
            known_facts: Vec::new(),
            messages: Vec::new(),
            unknown_mentioned: Vec::new(),
            clocks: Vec::new(),
            items: Vec::new(),
            meters: Vec::new(),
            habits: Vec::new(),
            exits: Vec::new(),
        });
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
}
