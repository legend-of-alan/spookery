//! What the model reads: the opening prompt of a turn and the replies its tool calls bring back.

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::json;

use crate::protocol::{
    self, CastKind, ItemStack, MenuRow, MeterView, NoteView, PackSummary, QueuedMove, Say,
    ServerBody, Speaker, PERCEIVE_BODY_CHARS,
};

/// How many tape lines the prompt's `buffer` keeps.
const RECENT_LINES: usize = 16;

/// One say as the model reads it in the prompt's `buffer` and `new_player_lines`, with the `seq` of the
/// message that carried it and without `recipients`. The GM's own lines carry the `seq` of
/// the server's reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TapeLine {
    pub seq: u64,
    pub from: Speaker,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    pub text: String,
}

impl TapeLine {
    /// The say a message with `seq` carried.
    pub fn new(seq: u64, say: Say) -> Self {
        Self {
            seq,
            from: say.from,
            address: say.address,
            place: say.place,
            actor: say.actor,
            text: say.text,
        }
    }
}

/// What starts a turn: a player's say, or a move in the queue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Wake {
    Say(TapeLine),
    Queued,
}

/// A player's view as `perceive` shows it to the model: who and what is here, no facts and no notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayerView {
    pub viewer: String,
    pub place: String,
    pub visible_actors: Vec<String>,
    pub meters: Vec<MeterView>,
    pub items: Vec<ItemStack>,
    pub habits: Vec<String>,
}

/// An NPC's view as `perceive` shows it to the model: a player's, plus its facts and its notes as
/// `messages`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NpcView {
    pub viewer: String,
    pub place: String,
    pub visible_actors: Vec<String>,
    pub facts: Vec<FactLine>,
    pub messages: Vec<NoteView>,
    pub meters: Vec<MeterView>,
    pub items: Vec<ItemStack>,
    pub habits: Vec<String>,
}

/// A known fact as the model reads it. Never `owner_body`: a type rule, not a filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FactLine {
    pub id: String,
    pub text: String,
}

impl From<protocol::View> for PlayerView {
    fn from(view: protocol::View) -> Self {
        Self {
            viewer: view.viewer,
            place: view.place,
            visible_actors: view.visible_actors,
            meters: view.meters,
            items: view.items,
            habits: view.habits,
        }
    }
}

impl From<protocol::View> for NpcView {
    fn from(view: protocol::View) -> Self {
        Self {
            viewer: view.viewer,
            place: view.place,
            visible_actors: view.visible_actors,
            facts: view
                .known_facts
                .into_iter()
                .map(|fact| FactLine {
                    id: fact.id,
                    text: fact.text,
                })
                .collect(),
            messages: view.notes,
            meters: view.meters,
            items: view.items,
            habits: view.habits,
        }
    }
}

/// Add `line` to the prompt's `buffer`, which keeps the latest `RECENT_LINES`.
pub fn keep_recent(buffer: &mut Vec<TapeLine>, line: TapeLine) {
    buffer.push(line);
    if buffer.len() > RECENT_LINES {
        buffer.drain(..buffer.len() - RECENT_LINES);
    }
}

/// What the model reads first on a turn: the menu, the recent tape, the new player lines,
/// the queue, what woke the turn and the actors present.
#[derive(Serialize)]
pub struct Opening<'a> {
    pub menu: &'a [MenuRow],
    pub buffer: &'a [TapeLine],
    pub new_player_lines: &'a [TapeLine],
    pub queue: &'a [QueuedMove],
    pub wake: &'a Wake,
    pub actors_present: &'a [String],
}

pub fn render_prompt(opening: &Opening) -> String {
    serde_json::to_string(opening).expect("prompt json")
}

/// One reply as the model reads it in a tool result: the body as sent, but for a view.
/// A view is a player's census, or an NPC's view with its facts and notes, plus the prose
/// of the place and of the viewer and everyone it sees, each cut at the perceive budget.
/// Fact bodies never come through.
pub fn render_reply(pack: &PackSummary, body: ServerBody) -> String {
    let ServerBody::View(view) = body else {
        return serde_json::to_string(&body).expect("server body");
    };
    let cut = |body: &String| body.chars().take(PERCEIVE_BODY_CHARS).collect::<String>();
    let place_body = pack.places.get(&view.place).map(cut).unwrap_or_default();
    let cast: BTreeMap<String, String> = std::iter::once(&view.viewer)
        .chain(&view.visible_actors)
        .filter_map(|id| Some((id.clone(), cut(pack.cast.get(id)?))))
        .collect();
    let mut seen = match pack.kinds.get(&view.viewer) {
        Some(CastKind::PcTemplate) => json!(PlayerView::from(*view)),
        _ => json!(NpcView::from(*view)),
    };
    seen["place_body"] = json!(place_body);
    seen["cast"] = json!(cast);
    seen.to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn docks() -> PackSummary {
        PackSummary {
            menu: Vec::new(),
            kinds: BTreeMap::from([
                ("aldric".into(), CastKind::Npc),
                ("bram".into(), CastKind::Npc),
                ("mira".into(), CastKind::Npc),
                ("pc_rowan".into(), CastKind::PcTemplate),
            ]),
            places: BTreeMap::from([("docks_shop".into(), "The shop smells of tar.".into())]),
            cast: BTreeMap::from([
                ("aldric".into(), "Aldric keeps the counter.".into()),
                ("bram".into(), "BRAM-ABSENT".into()),
                ("mira".into(), "MIRA-PROSE-SENTINEL".into()),
                ("pc_rowan".into(), "Rowan came in from the rain.".into()),
            ]),
        }
    }

    /// A shop view by `viewer` that sees Aldric, Mira and Rowan, with one hidden fact, one
    /// note and one unknown mention.
    fn shop(viewer: &str) -> protocol::View {
        serde_json::from_value(json!({
            "viewer": viewer,
            "place": "docks_shop",
            "visible_actors": ["aldric", "mira", "pc_rowan"],
            "known_facts": [{"id": "f", "text": "FACT-TEXT-SENTINEL", "owner_body": "BODY-SENTINEL"}],
            "notes": [{"id": "m", "to": [], "fact_ids": [], "text": "MESSAGE-SENTINEL"}],
            "unknown_mentions": ["UNKNOWN-SENTINEL"],
            "items": [],
            "habits": []
        }))
        .unwrap()
    }

    /// Feature: The AI GM gets non-hidden prose
    ///
    /// Scenario: Cast prose reaches the perceive result, and hidden facts never do
    ///   Given a hidden fact body "BODY-SENTINEL"
    ///   Then the result holds the place's and the seen cast's bodies and the fact text
    ///   And it does not contain "BODY-SENTINEL"
    #[test]
    fn an_npc_view_keeps_prose_and_fact_text_and_never_a_fact_body() {
        let seen = render_reply(&docks(), ServerBody::View(Box::new(shop("aldric"))));
        assert!(seen.contains("MIRA-PROSE-SENTINEL"), "{seen}");
        assert!(seen.contains("The shop smells of tar."), "{seen}");
        assert!(seen.contains("Aldric keeps the counter."), "{seen}");
        assert!(!seen.contains("BRAM-ABSENT"), "{seen}");
        assert!(seen.contains("FACT-TEXT-SENTINEL"), "{seen}");
        assert!(seen.contains("MESSAGE-SENTINEL"), "{seen}");
        assert!(!seen.contains("BODY-SENTINEL"), "{seen}");
        assert!(!seen.contains("UNKNOWN-SENTINEL"), "{seen}");
    }

    #[test]
    fn a_long_body_is_cut_at_the_budget() {
        let mut pack = docks();
        let budget = PERCEIVE_BODY_CHARS;
        pack.cast.insert("aldric".into(), "Z".repeat(budget + 1));
        pack.places
            .insert("docks_shop".into(), "Y".repeat(budget + 1));
        let seen = render_reply(&pack, ServerBody::View(Box::new(shop("mira"))));
        for letter in ["Z", "Y"] {
            assert!(seen.contains(&letter.repeat(budget)), "{seen}");
            assert!(!seen.contains(&letter.repeat(budget + 1)), "{seen}");
        }
    }

    #[test]
    fn a_player_view_is_a_census_without_facts_or_notes() {
        let seen = render_reply(&docks(), ServerBody::View(Box::new(shop("pc_rowan"))));
        assert!(seen.contains("MIRA-PROSE-SENTINEL"), "{seen}");
        for hidden in [
            "FACT-TEXT",
            "BODY-SENTINEL",
            "MESSAGE-SENTINEL",
            "UNKNOWN-SENTINEL",
        ] {
            assert!(!seen.contains(hidden), "{seen}");
        }
    }

    #[test]
    fn the_buffer_keeps_sixteen_and_drops_the_oldest() {
        let mut buffer = Vec::new();
        for index in 0..20 {
            let line = TapeLine {
                seq: index,
                from: Speaker::Player("pc_rowan".to_string()),
                address: "table".to_string(),
                place: None,
                actor: None,
                text: index.to_string(),
            };
            keep_recent(&mut buffer, line);
        }
        assert_eq!(buffer.len(), RECENT_LINES);
        assert_eq!(buffer[0].text, "4");
        assert_eq!(buffer[15].text, "19");
    }
}
