//! The GM's tools: what the model may call, how the agent answers each call, and the result
//! the model reads back.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Serialize;
use serde_json::{json, Value};

use crate::protocol::{
    ControlType, ItemStack, MeterView, NoteView, PackSummary, Reason, ServerBody, CLIENT_SCHEMA,
    PERCEIVE_BODY_CHARS,
};
use crate::{Agent, Connection, Model, ToolCall};

/// The tools that send the GM command of the same name.
const COMMAND_TOOLS: [&str; 3] = ["perceive", "say", "queue"];

/// The tool the agent answers with `commit`.
const END_TURN: &str = "end_turn";

/// The GM's tools: the `COMMAND_TOOLS`, with parameters from the client schema, and `END_TURN`.
pub(crate) static TOOLS: LazyLock<Value> = LazyLock::new(|| {
    let schema: Value = serde_json::from_str(CLIENT_SCHEMA).expect("client schema");
    let tool = |name: &str, description: &str, parameters: &Value| {
        json!({
            "type": "function",
            "function": { "name": name, "description": description, "parameters": parameters }
        })
    };
    let args = |name: &str| &schema["$defs"][name];
    let [perceive, say, queue] = COMMAND_TOOLS;
    json!([
        tool(
            perceive,
            "See what one actor sees now: where they are, who is there, what they carry and know. `as` is the actor's id.",
            args("PerceiveArgs"),
        ),
        tool(
            say,
            "Speak to everyone (address `table`), to everyone in one place, or to one actor. No game time passes.",
            args("SayArgs"),
        ),
        tool(
            queue,
            "Queue a move from the menu for an actor. The server answers queued or refused.",
            args("QueueArgs"),
        ),
        tool(
            END_TURN,
            "Play the round: the engine resolves every queued move and answers with the rolls. You may say what happened afterwards. Answer with no tool calls when you are done.",
            &json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ])
});

/// Answers `tool_call`: the server's reply to the command it asks for, or the agent's own
/// answer when no command goes out.
pub(crate) async fn call<M: Model, C: Connection>(
    agent: &Agent<M, C>,
    tool_call: &ToolCall,
) -> graph_flow::Result<String> {
    let args = match tool_call.arguments.trim() {
        "" => Some(json!({})),
        text => serde_json::from_str(text).ok().filter(Value::is_object),
    };
    let body = match (tool_call.name.as_str(), args) {
        (name, Some(args)) if COMMAND_TOOLS.contains(&name) => {
            agent.send_command(name, args).await?
        }
        (END_TURN, Some(_)) => match agent.commit_queued_moves().await? {
            Some(body) => body,
            None => return Ok("nothing is queued, so there is no round to play".into()),
        },
        // Arguments that are not a JSON object, or a tool the GM does not have: the table
        // refuses a GM command it cannot read the same way.
        _ => ServerBody::Refused(Reason::Schema.into()),
    };
    Ok(render_result(&agent.state().pack, body))
}

/// One reply as the model reads it in a tool result: the body as sent, but for a view.
fn render_result(pack: &PackSummary, body: ServerBody) -> String {
    let ServerBody::View(view) = body else {
        return serde_json::to_string(&body).expect("server body");
    };
    let cut = |body: &String| body.chars().take(PERCEIVE_BODY_CHARS).collect::<String>();
    let cast = std::iter::once(&view.viewer)
        .chain(&view.visible_actors)
        .filter_map(|id| Some((id.clone(), cut(pack.cast.get(id)?))))
        .collect();
    let npc = pack.control_types.get(&view.viewer) != Some(&ControlType::PcTemplate);
    let facts = view.known_facts.into_iter().map(|fact| FactLine {
        id: fact.id,
        text: fact.text,
    });
    let result = PerceiveResult {
        place_body: pack.places.get(&view.place).map(cut).unwrap_or_default(),
        viewer: view.viewer,
        place: view.place,
        visible_actors: view.visible_actors,
        facts: npc.then(|| facts.collect()),
        notes: npc.then_some(view.notes),
        meters: view.meters,
        items: view.items,
        habits: view.habits,
        cast,
    };
    serde_json::to_string(&result).expect("perceive result")
}

/// A view as `perceive` shows it to the model: who and what is here, an NPC's facts and notes
/// (a player's view has neither), and the prose of the place and of the viewer and everyone it
/// sees, each cut at the perceive budget.
#[derive(Serialize)]
struct PerceiveResult {
    viewer: String,
    place: String,
    visible_actors: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    facts: Option<Vec<FactLine>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notes: Option<Vec<NoteView>>,
    meters: Vec<MeterView>,
    items: Vec<ItemStack>,
    habits: Vec<String>,
    place_body: String,
    cast: BTreeMap<String, String>,
}

/// A known fact as the model reads it. Never `owner_body`: a type rule, not a filter.
#[derive(Serialize)]
struct FactLine {
    id: String,
    text: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol;

    fn docks() -> PackSummary {
        PackSummary {
            menu: Vec::new(),
            control_types: BTreeMap::from([
                ("aldric".into(), ControlType::Npc),
                ("bram".into(), ControlType::Npc),
                ("mira".into(), ControlType::Npc),
                ("rowan".into(), ControlType::PcTemplate),
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
