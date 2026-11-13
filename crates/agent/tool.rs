//! The GM's tools: what the model may call, how the agent answers each call, and the result
//! the model reads back.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::protocol::{
    self, CastKind, CommitArgs, ItemStack, MeterView, NoteView, PackSummary, ServerBody,
    CLIENT_SCHEMA, PERCEIVE_BODY_CHARS,
};
use crate::{Agent, Connection, Model, ToolCall};

/// The GM's tools: the GM commands `perceive`, `say` and `queue`, their parameters read from
/// the published client schema, and `end_turn`, which the agent answers with `commit`.
pub(crate) static TOOLS: LazyLock<Value> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(CLIENT_SCHEMA).expect("client schema");
    let tool = |name: &str, description: &str, parameters: &Value| {
        json!({
            "type": "function",
            "function": { "name": name, "description": description, "parameters": parameters }
        })
    };
    let args = |name: &str| &schema["$defs"][name];
    json!([
        tool(
            "perceive",
            "See what one actor sees now: where they are, who is there, what they carry and know. `as` is the actor's id.",
            args("PerceiveArgs"),
        ),
        tool(
            "say",
            "Speak to everyone (address `table`), to everyone in one place, or to one actor. No game time passes.",
            args("SayArgs"),
        ),
        tool(
            "queue",
            "Queue a move from the menu for an actor. The server answers queued or refused.",
            args("QueueArgs"),
        ),
        tool(
            "end_turn",
            "Play the round: the engine resolves every queued move and answers with the rolls. You may say what happened afterwards. Answer with no tool calls when you are done.",
            &json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ])
});

/// `commit` with its rounds, as the `end_turn` tool and `FinishTurn` send it.
pub(crate) const COMMIT_ROUND: CommitArgs = CommitArgs {
    include_round: true,
    session_id: None,
};

/// Answers `tool_call`: the server's reply to the command it asks for, or the agent's own
/// answer when no command goes out.
pub(crate) async fn call<M: Model, C: Connection>(
    agent: &Agent<M, C>,
    tool_call: &ToolCall,
) -> graph_flow::Result<String> {
    let args = match tool_call.arguments.trim() {
        "" => Map::new(),
        text => match serde_json::from_str(text) {
            Ok(Value::Object(args)) => args,
            _ => return Ok("refused: the arguments are not a JSON object".to_string()),
        },
    };
    let (command, args) = match tool_call.name.as_str() {
        "perceive" | "say" | "queue" => (tool_call.name.as_str(), Value::Object(args)),
        "end_turn" if !agent.state().queued_moves.is_empty() => ("commit", json!(COMMIT_ROUND)),
        "end_turn" => return Ok("nothing is queued, so there is no round to play".into()),
        name => return Ok(format!("refused: there is no tool named {name}")),
    };
    let body = agent.send_command(command, args).await?;
    Ok(render_result(&agent.state().pack, body))
}

/// One reply as the model reads it in a tool result: the body as sent, but for a view.
/// A view is a player's census, or an NPC's view with its facts and notes, plus the prose
/// of the place and of the viewer and everyone it sees, each cut at the perceive budget.
/// Fact bodies never come through.
fn render_result(pack: &PackSummary, body: ServerBody) -> String {
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

/// A player's view as `perceive` shows it to the model: who and what is here, no facts and no notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PlayerView {
    viewer: String,
    place: String,
    visible_actors: Vec<String>,
    meters: Vec<MeterView>,
    items: Vec<ItemStack>,
    habits: Vec<String>,
}

/// An NPC's view as `perceive` shows it to the model: a player's, plus its facts and its notes as
/// `messages`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct NpcView {
    viewer: String,
    place: String,
    visible_actors: Vec<String>,
    facts: Vec<FactLine>,
    messages: Vec<NoteView>,
    meters: Vec<MeterView>,
    items: Vec<ItemStack>,
    habits: Vec<String>,
}

/// A known fact as the model reads it. Never `owner_body`: a type rule, not a filter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct FactLine {
    id: String,
    text: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn docks() -> PackSummary {
        PackSummary {
            menu: Vec::new(),
            kinds: BTreeMap::from([
                ("aldric".into(), CastKind::Npc),
                ("bram".into(), CastKind::Npc),
                ("mira".into(), CastKind::Npc),
                ("rowan".into(), CastKind::PcTemplate),
            ]),
            places: BTreeMap::from([("docks_shop".into(), "The shop smells of tar.".into())]),
            cast: BTreeMap::from([
                ("aldric".into(), "Aldric keeps the counter.".into()),
                ("bram".into(), "BRAM-ABSENT".into()),
                ("mira".into(), "MIRA-PROSE-SENTINEL".into()),
                ("rowan".into(), "Rowan came in from the rain.".into()),
            ]),
        }
    }

    /// A shop view by `viewer` that sees Aldric, Mira and Rowan, with one hidden fact, one
    /// note and one unknown mention.
    fn shop(viewer: &str) -> protocol::View {
        serde_json::from_value(json!({
            "viewer": viewer,
            "place": "docks_shop",
            "visible_actors": ["aldric", "mira", "rowan"],
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
        let seen = render_result(&docks(), ServerBody::View(Box::new(shop("aldric"))));
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
        let seen = render_result(&pack, ServerBody::View(Box::new(shop("mira"))));
        for letter in ["Z", "Y"] {
            assert!(seen.contains(&letter.repeat(budget)), "{seen}");
            assert!(!seen.contains(&letter.repeat(budget + 1)), "{seen}");
        }
    }

    #[test]
    fn a_player_view_is_a_census_without_facts_or_notes() {
        let seen = render_result(&docks(), ServerBody::View(Box::new(shop("rowan"))));
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
}
