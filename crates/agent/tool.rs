//! The GM's tools: what the model may call, how the agent answers each call, and the result
//! the model reads back.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Serialize;
use serde_json::{json, Value};

use crate::protocol::{
    self, ItemStack, MeterView, NoteView, PackSummary, Reason, ServerBody, CLIENT_SCHEMA,
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
    let tool = |tool_id: &str, description: &str, parameters: &Value| {
        json!({
            "type": "function",
            "function": { "name": tool_id, "description": description, "parameters": parameters }
        })
    };
    let args = |type_id: &str| &schema["$defs"][type_id];
    let [perceive, say, queue] = COMMAND_TOOLS;
    json!([
        tool(
            perceive,
            "See what one actor sees now: where they are, who is there, what they carry and know. `as` is the actor's id.",
            args("PerceiveArgs"),
        ),
        tool(
            say,
            "Speak to everyone (`to` is `table`), to everyone in one place (`{\"place\": id}`), or to one actor (`{\"actor\": id}`). `as` is an NPC's id to speak as that NPC; without `to`, that NPC's place hears it. No game time passes.",
            args("SayArgs"),
        ),
        tool(
            queue,
            "Queue a move from the menu for an actor. The server answers queued or refused.",
            args("QueueArgs"),
        ),
        tool(
            END_TURN,
            "Play the round: the engine resolves every queued move and answers with the rolls.",
            &json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ])
});

/// Answers `tool_call`: the server's reply to the command it asks for, or the agent's own
/// answer when no command goes out.
pub(crate) async fn call<M: Model, Conn: Connection>(
    agent: &Agent<M, Conn>,
    tool_call: &ToolCall,
) -> graph_flow::Result<String> {
    tracing::debug!(tool_id = %tool_call.tool_id, "tool call");
    let args = match tool_call.arguments.trim() {
        "" => Some(json!({})),
        text => serde_json::from_str(text).ok().filter(Value::is_object),
    };
    let body = match (tool_call.tool_id.as_str(), args) {
        (tool_id, Some(args)) if COMMAND_TOOLS.contains(&tool_id) => {
            agent.send_command(tool_id, args).await?
        }
        (END_TURN, Some(_)) => match agent.commit_queued_moves().await? {
            Some(body) => body,
            None => return Ok("nothing is queued, so there is no round to play".into()),
        },
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
    let npc = pack.control_types.get(&view.viewer) != Some(&protocol::ControlType::PcTemplate);
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

    /// No pack's prose runs past the perceive budget, so no scenario reaches the cut.
    #[test]
    fn a_long_body_is_cut_at_the_budget() {
        let budget = PERCEIVE_BODY_CHARS;
        let pack = PackSummary {
            places: BTreeMap::from([("docks_shop".into(), "Y".repeat(budget + 1))]),
            cast: BTreeMap::from([("aldric".into(), "Z".repeat(budget + 1))]),
            ..PackSummary::default()
        };
        let view = serde_json::from_value(json!({
            "viewer": "mira", "place": "docks_shop", "visible_actors": ["aldric", "mira"],
            "known_facts": [], "notes": [], "unknown_mentions": [], "items": [], "habits": []
        }))
        .unwrap();
        let seen = render_result(&pack, ServerBody::View(Box::new(view)));
        for letter in ["Z", "Y"] {
            assert!(seen.contains(&letter.repeat(budget)), "{seen}");
            assert!(!seen.contains(&letter.repeat(budget + 1)), "{seen}");
        }
    }
}
