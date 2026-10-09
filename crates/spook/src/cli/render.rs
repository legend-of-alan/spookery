use crate::protocol::{ClockView, FactNote, KnownFact, PlayerMessage, StockView};

/// Readable text for one player frame. A new [`PlayerMessage`] variant fails here until it is rendered.
pub fn render(message: &PlayerMessage) -> String {
    match message {
        PlayerMessage::Say {
            from,
            address: _,
            text,
        } => format!("{}: {text}", speaker(from)),
        PlayerMessage::Queued { id: _ } => "Your action is queued.".to_string(),
        PlayerMessage::Check { success } => {
            if *success {
                "Your roll succeeded.".to_string()
            } else {
                "Your roll failed.".to_string()
            }
        }
        PlayerMessage::Resume { text } => resume(text),
        PlayerMessage::Refused { code } => refusal(code),
        PlayerMessage::Perceive {
            viewer,
            place,
            visible_actors,
            known_facts,
            messages,
            unknown_mentioned: _,
            clocks,
            stocks,
            habits: _,
        } => perceive(
            viewer,
            place,
            visible_actors,
            known_facts,
            messages,
            clocks,
            stocks,
        ),
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
        "rumor" => match rest.split_whitespace().collect::<Vec<_>>()[..] {
            [_rumor, fact] => format!("You heard a rumor: {}.", humanize(fact)),
            _ => row.to_string(),
        },
        "clock" => match rest.split_whitespace().collect::<Vec<_>>()[..] {
            [id, value, max, state @ ("paused" | "running")] => {
                match (value.parse(), max.parse()) {
                    (Ok(value), Ok(max)) => clock_line(&ClockView {
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

fn refusal(code: &str) -> String {
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
        "NOT_IN_SPEC" => "The table does not understand that.",
        "SESSION_FULL" => "The table is full.",
        "MISSING_ITEM" => "You do not have that.",
        "SKIPPED" => "That action was skipped.",
        _ => return format!("The table refused that. [{code}]"),
    };
    format!("{sentence} [{code}]")
}

fn perceive(
    viewer: &str,
    place: &str,
    visible_actors: &[String],
    known_facts: &[KnownFact],
    messages: &[FactNote],
    clocks: &[ClockView],
    stocks: &[StockView],
) -> String {
    let mut sections = Vec::new();
    if !place.is_empty() {
        sections.push(format!("You are in {}.", humanize(place)));
    }
    if !visible_actors.is_empty() {
        let names = visible_actors
            .iter()
            .map(|id| {
                if is_self(id, viewer) {
                    "You".to_string()
                } else {
                    humanize(id)
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
    if !stocks.is_empty() {
        let mut lines = Vec::new();
        for stock in stocks {
            lines.push(format!("{}: {}", humanize(&stock.sku), stock.qty));
        }
        sections.push(format!("Stock:\n{}", lines.join("\n")));
    }
    sections.join("\n\n")
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

fn clock_line(clock: &ClockView) -> String {
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

/// `docks_shop` becomes "Docks shop". Role prefixes such as `npc:` are not part of the name.
fn humanize(id: &str) -> String {
    let bare = id.strip_prefix("npc:").unwrap_or(id);
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

    fn assert_readable(text: &str) {
        assert!(!text.contains('{'), "{text}");
        assert!(!text.contains("\"kind\""), "{text}");
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
        let yes = render(&PlayerMessage::Check { success: true });
        let no = render(&PlayerMessage::Check { success: false });
        assert_eq!(yes, "Your roll succeeded.");
        assert_eq!(no, "Your roll failed.");
        assert_readable(&yes);
        assert_readable(&no);
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
rumor rumor_cellar fact_cellar_door
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
You heard a rumor: Fact cellar door.
Assassination debt: 2/6, paused
Shop day: 1/1"
        );
        assert!(!text.contains("note_mira_door"), "{text}");
        assert!(!text.contains("rumor_cellar"), "{text}");
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
        });
        assert_eq!(full, "The table is full. [SESSION_FULL]");
        assert_readable(&full);

        let unknown = render(&PlayerMessage::Refused {
            code: "NEW_CODE".to_string(),
        });
        assert_eq!(unknown, "The table refused that. [NEW_CODE]");
        assert_readable(&unknown);
    }

    #[test]
    fn a_perceive_names_the_place_and_skips_empty_sections() {
        let text = render(&PlayerMessage::Perceive {
            viewer: "player:pc_rowan".to_string(),
            place: "docks_shop".to_string(),
            visible_actors: vec!["aldric".to_string(), "pc_rowan".to_string()],
            known_facts: vec![KnownFact {
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
            stocks: vec![StockView {
                sku: "lamp_oil".to_string(),
                qty: 4,
            }],
            habits: vec!["tend_shop".to_string()],
        });
        assert_eq!(
            text,
            "\
You are in Docks shop.

Here: Aldric, You

You know:
Oil is on the shelf.

Notes:
Back in an hour.

Clocks:
Assassination debt: 2/6, paused
Shop day: 1/1

Stock:
Lamp oil: 4"
        );
        assert!(!text.contains("CANON-BODY"), "{text}");
        assert!(!text.contains("secret_fact"), "{text}");
        assert!(!text.contains("tend_shop"), "{text}");
        assert!(!text.contains("note_door"), "{text}");
        assert!(!text.contains("docks_shop"), "{text}");
        assert_readable(&text);

        let quiet = render(&PlayerMessage::Perceive {
            viewer: "player:pc_rowan".to_string(),
            place: "docks_street".to_string(),
            visible_actors: vec!["bram".to_string()],
            known_facts: Vec::new(),
            messages: Vec::new(),
            unknown_mentioned: Vec::new(),
            clocks: Vec::new(),
            stocks: Vec::new(),
            habits: Vec::new(),
        });
        assert_eq!(quiet, "You are in Docks street.\n\nHere: Bram");
        assert!(!quiet.contains("You know:"), "{quiet}");
        assert!(!quiet.contains("Notes:"), "{quiet}");
        assert!(!quiet.contains("Clocks:"), "{quiet}");
        assert!(!quiet.contains("Stock:"), "{quiet}");
        assert_readable(&quiet);
    }
}
