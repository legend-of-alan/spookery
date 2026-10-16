use crate::protocol::{Outcome, PlayerMessage, WireClock, WireExit, WireMeter};

/// Readable text for one player frame. A new [`PlayerMessage`] variant fails here until it is rendered.
pub fn render(message: &PlayerMessage) -> String {
    match message {
        PlayerMessage::Say {
            from,
            address: _,
            text,
        } => format!("{}: {text}", speaker(from)),
        PlayerMessage::Queued { id: _ } => "Your action is queued.".to_string(),
        PlayerMessage::Check { outcome } => match outcome {
            Outcome::Success => "Your roll succeeded.".to_string(),
            Outcome::Partial => "Your roll partly succeeded.".to_string(),
            Outcome::Miss => "Your roll missed.".to_string(),
        },
        PlayerMessage::Resume { text } => resume(text),
        PlayerMessage::Lines { text } => text.clone(),
        PlayerMessage::Refused { code, missing } => named_refusal(code, missing.as_deref()),
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

fn resume(text: &str) -> String {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|row| !row.is_empty())
        .map(resume_row)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        "While you were away:".to_string()
    } else {
        format!("While you were away:\n{}", lines.join("\n"))
    }
}

/// One player digest row as a sentence. A row this does not recognize is shown as it came.
fn resume_row(row: &str) -> String {
    let (kind, rest) = row.split_once(' ').unwrap_or((row, ""));
    match kind {
        "message" => match rest.split_once(' ') {
            Some((_id, text)) if !text.trim().is_empty() => text.trim().to_string(),
            _ => row.to_string(),
        },
        "clock" => match rest.split_whitespace().collect::<Vec<_>>()[..] {
            [id, value, max, state @ ("paused" | "running")] => {
                match (value.parse(), max.parse()) {
                    (Ok(value), Ok(max)) => clock_line(&WireClock {
                        id: id.to_string(),
                        value,
                        max,
                        paused: state == "paused",
                    }),
                    _ => row.to_string(),
                }
            }
            _ => row.to_string(),
        },
        _ => row.to_string(),
    }
}

pub(crate) fn refusal(code: &str) -> String {
    named_refusal(code, None)
}

fn named_refusal(code: &str, missing: Option<&str>) -> String {
    let sentence = match code {
        "ILLEGAL_CLASS" => "That action is not allowed.",
        "ILLEGAL_KNOWLEDGE" => "You cannot know that.",
        "OUT_OF_RANGE" => "That is out of range.",
        "BUDGET" => "That does not fit.",
        "PAUSED_CLOCK" => "That clock is paused.",
        "UNKNOWN_KEY" => "That is not in the world.",
        "VIEWER_MISMATCH" => "That is not your view.",
        "KERNEL_ON_PLAYER" => "The table cannot do that for you.",
        "INELIGIBLE" => "You cannot do that right now.",
        "NO_PROFILE" => "There is no profile for that.",
        "ACTOR_MISMATCH" => "That action is not yours.",
        "TABLE_EMPTY" => "Nobody is seated.",
        "PACK_REV_MISMATCH" => "The save does not match this pack.",
        "UNKNOWN_MOVE" => "That is not a move.",
        "UNSCOPED_PERCEIVE" => "You cannot see that.",
        "COMMIT_CONFLICT" => "The table was already writing. Try again.",
        "HAND_EDIT" => "The save was edited by hand.",
        "SCHEMA" => "That line is not valid.",
        "NOT_IN_SPEC" => "The table does not understand that. Type help.",
        "SESSION_FULL" => "The table is full.",
        "SEAT_TAKEN" => "That character is already taken.",
        "MISSING_ITEM" => "You do not have that.",
        "NOT_A_WEAPON" => "That is not a weapon.",
        "LINE_TOO_LONG" => "That line is too long.",
        "NO_EXIT" => {
            return match missing {
                Some(text) if !text.is_empty() => format!("{text} [{code}]"),
                _ => format!("There is no exit that way. [{code}]"),
            };
        }
        "BUSY" => return format!("You are busy. [{code}]"),
        "CANT_AFFORD" => {
            return match missing {
                Some(what) if !what.is_empty() => format!("You cannot afford {what}. [{code}]"),
                _ => format!("You cannot afford that. [{code}]"),
            };
        }
        _ => return format!("The table refused that. [{code}]"),
    };
    format!("{sentence} [{code}]")
}

fn view_text(message: &PlayerMessage) -> String {
    let PlayerMessage::View {
        viewer,
        place,
        shown,
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
    let titled = if !shown.place_name.is_empty() {
        shown.place_name.as_str()
    } else {
        place.as_str()
    };
    if !titled.is_empty() {
        sections.push(format!("You are in {titled}."));
    }
    if !shown.status.is_empty() {
        sections.push(shown.status.clone());
    }
    let prose = shown.prose.trim();
    if !prose.is_empty() {
        sections.push(prose.to_string());
    }
    if !visible_actors.is_empty() {
        let names = visible_actors
            .iter()
            .enumerate()
            .map(|(index, id)| {
                if is_self(id, viewer) {
                    "You".to_string()
                } else {
                    shown
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
        .map(|exit| spookery_kernel::api::map::exit_phrase(&exit.name, exit.minutes, exit.locked))
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

fn is_self(id: &str, viewer: &str) -> bool {
    match viewer.strip_prefix("player:") {
        Some(actor) if !actor.is_empty() => id == actor || id == viewer,
        _ => id == viewer,
    }
}

/// `docks_shop` becomes "Docks shop". `npc:` and `player:` are not part of the name.
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
    use crate::protocol::{FactNote, WireFact, WireItem};

    fn assert_readable(text: &str) {
        assert!(!text.contains('{'), "{text}");
        assert!(!text.contains("\"kind\""), "{text}");
    }

    #[test]
    fn a_player_prefix_is_not_part_of_the_name() {
        assert_eq!(humanize("player:pc_rowan"), "Pc rowan");
        assert_eq!(humanize("npc:mira"), "Mira");
        assert_eq!(humanize("docks_shop"), "Docks shop");
    }

    #[test]
    fn a_say_is_a_name_and_the_text() {
        let mira = render(&PlayerMessage::Say {
            from: "npc:mira".to_string(),
            address: "table".to_string(),
            text: "the lamp is lit".to_string(),
        });
        assert_eq!(mira, "Mira: the lamp is lit");
        assert_readable(&mira);

        let gm = render(&PlayerMessage::Say {
            from: "gm".to_string(),
            address: "table".to_string(),
            text: "evening".to_string(),
        });
        assert_eq!(gm, "GM: evening");
        assert_readable(&gm);

        let you = render(&PlayerMessage::Say {
            from: "player:pc_rowan".to_string(),
            address: "actor".to_string(),
            text: "hello".to_string(),
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

    #[test]
    fn a_resume_is_an_away_block() {
        let text = render(&PlayerMessage::Resume {
            text: "the shop restocked\n".to_string(),
        });
        assert_eq!(text, "While you were away:\nthe shop restocked");
        assert_readable(&text);
        assert_eq!(
            render(&PlayerMessage::Resume {
                text: String::new(),
            }),
            "While you were away:"
        );
    }

    #[test]
    fn a_resume_turns_digest_rows_into_sentences() {
        let text = render(&PlayerMessage::Resume {
            text: "\
message note_mira_door Back at dusk. Oil on the shelf.
clock assassination_debt 2 6 paused
clock shop_day 1 1 running
"
            .to_string(),
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
    fn a_resume_row_it_cannot_read_is_shown_as_it_came() {
        let text = render(&PlayerMessage::Resume {
            text: "clock broken two six paused\nmessage lonely_id\nweather fog\n".to_string(),
        });
        assert_eq!(
            text,
            "While you were away:\nclock broken two six paused\nmessage lonely_id\nweather fog"
        );
    }

    #[test]
    fn a_refusal_is_a_sentence_with_the_code() {
        let full = render(&PlayerMessage::Refused {
            code: "SESSION_FULL".to_string(),
            missing: None,
        });
        assert_eq!(full, "The table is full. [SESSION_FULL]");
        assert_readable(&full);
        let taken = render(&PlayerMessage::Refused {
            code: "SEAT_TAKEN".to_string(),
            missing: None,
        });
        assert_eq!(taken, "That character is already taken. [SEAT_TAKEN]");
        assert_readable(&taken);
        let long = render(&PlayerMessage::Refused {
            code: "LINE_TOO_LONG".to_string(),
            missing: None,
        });
        assert_eq!(long, "That line is too long. [LINE_TOO_LONG]");
        assert_readable(&long);

        let unknown = render(&PlayerMessage::Refused {
            code: "NEW_CODE".to_string(),
            missing: None,
        });
        assert_eq!(unknown, "The table refused that. [NEW_CODE]");
        assert_readable(&unknown);

        let afford = render(&PlayerMessage::Refused {
            code: "CANT_AFFORD".to_string(),
            missing: Some("grit".to_string()),
        });
        assert_eq!(afford, "You cannot afford grit. [CANT_AFFORD]");
        assert_readable(&afford);
        let wire = serde_json::to_string(&PlayerMessage::Refused {
            code: "CANT_AFFORD".to_string(),
            missing: Some("grit".to_string()),
        })
        .unwrap();
        assert_eq!(
            wire,
            r#"{"kind":"refused","code":"CANT_AFFORD","missing":"grit"}"#
        );
        let bare = serde_json::to_string(&PlayerMessage::Refused {
            code: "SCHEMA".to_string(),
            missing: None,
        })
        .unwrap();
        assert_eq!(bare, r#"{"kind":"refused","code":"SCHEMA"}"#);
    }

    #[test]
    fn a_perceive_names_the_place_and_skips_empty_sections() {
        let text = render(&PlayerMessage::View {
            viewer: "player:pc_rowan".to_string(),
            place: "docks_shop".to_string(),
            shown: Box::new(crate::protocol::Shown {
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
            messages: vec![FactNote {
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
            viewer: "player:pc_rowan".to_string(),
            place: "docks_street".to_string(),
            shown: Box::new(crate::protocol::Shown {
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
