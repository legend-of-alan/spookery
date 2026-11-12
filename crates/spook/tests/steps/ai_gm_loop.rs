//! The AI GM's turn loop against a real table: perceive first, recover from a refusal,
//! queue several NPCs, stop at the step limit, and outwait the graph's task timeout.

use std::time::Duration;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery::agent::{ChatMessage, MAX_TOOL_CALLS};
use spookery_engine::testing::journal::{list_round_keys, read_record_line};
use spookery_engine::testing::state_of;

use crate::support::copy_docks;
use crate::World;

use super::scripted_gm::{read_tool_results, start_ai_gm};
use super::table;

/// graph-flow's default task timeout, which the turn graph keeps.
const GRAPH_TASK_TIMEOUT: Duration = Duration::from_secs(300);

const END_TURN: (&str, &str) = ("end_turn", "{}");
const QUEUE_MIRA_WAIT: (&str, &str) = ("queue", r#"{"actor_id":"mira","move":"wait"}"#);
const NARRATION: &str = "The street opens past the shop door.";

#[given(expr = "a docks table with a seated player and the NPC Mira in the shop")]
#[given(expr = "a docks table with a seated player and the NPCs Mira and Aldric in the shop")]
fn a_docks_table_with_npcs_in_the_shop(world: &mut World) {
    let pack = copy_docks("ai-gm-loop");
    // Aldric starts in the shop. Mira starts upstairs.
    super::common::edit_save(&pack, |now| {
        now.actors_mut()
            .set_place(&"mira".into(), "docks_shop")
            .unwrap();
    });
    let state = state_of(&pack);
    for npc in ["mira", "aldric"] {
        let place = state.actors().stores().place.get(npc).map(String::as_str);
        assert_eq!(place, Some("docks_shop"), "{npc}");
    }
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a seated player who has queued a move")]
fn a_docks_table_with_a_queued_move(world: &mut World) {
    let (pack, _) = super::ai_gm::queued_travel(world, "ai-gm-stuck");
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a seated player and the AI GM connected")]
fn a_docks_table_with_the_ai_gm_sitting(world: &mut World) {
    let pack = copy_docks("ai-gm-quiet");
    table::listen(world, &pack);
    table::seat_rowan(world, &pack);
    world.pack = Some(pack);
    start_ai_gm_once(world);
}

#[given(
    expr = "a scripted model that perceives as Mira, then has Mira say hello, then ends the turn"
)]
fn a_model_that_perceives_first(world: &mut World) {
    world.model.push(&[("perceive", r#"{"as":"mira"}"#)]);
    world
        .model
        .push(&[("say", r#"{"address":"table","text":"Mira: hello"}"#)]);
    world.model.push(&[END_TURN]);
}

#[given(expr = "a scripted model that queues a move Mira can't make")]
fn a_model_that_queues_a_bad_move(world: &mut World) {
    world
        .model
        .push(&[("queue", r#"{"actor_id":"mira","move":"fly"}"#)]);
    world.model.push(&[QUEUE_MIRA_WAIT]);
    world.model.push(&[END_TURN]);
}

#[given(
    expr = "a scripted model that queues a move for Mira and a move for Aldric, then ends the turn"
)]
fn a_model_that_queues_two_npcs(world: &mut World) {
    world.model.push(&[
        QUEUE_MIRA_WAIT,
        ("queue", r#"{"actor_id":"aldric","move":"wait"}"#),
    ]);
    world.model.push(&[END_TURN]);
    let say = format!(r#"{{"address":"table","text":"{NARRATION}"}}"#);
    world.model.push(&[("say", &say)]);
}

#[given(expr = "a scripted model that only ever calls perceive")]
fn a_model_that_only_perceives(world: &mut World) {
    world.model.repeat(&[("perceive", r#"{"as":"rowan"}"#)]);
}

/// Starts the AI GM with the scenario's model, unless it already runs.
fn start_ai_gm_once(world: &mut World) {
    if world.ai_gm.is_none() {
        world.ai_gm = Some(start_ai_gm(world, world.model.clone()));
    }
}

#[when(expr = "the player says {string} in the shop")]
fn the_player_says(world: &mut World, text: String) {
    player_says(world, &text);
}

/// The player's say wakes the turn whose queue the table refuses.
#[when(expr = "the table refuses the queue")]
fn the_table_refuses_the_queue(world: &mut World) {
    player_says(world, "evening");
}

/// Starts the AI GM, has the player say `text`, and waits until the turn it woke is done.
fn player_says(world: &mut World, text: &str) {
    start_ai_gm_once(world);
    world.mark = table::player_mark(world);
    table::player_send(world, &format!("say {text}"), "\"kind\":\"say\"");
    world.model.wait_past_script();
}

#[when(expr = "the player queues travel")]
fn the_player_queues_travel(world: &mut World) {
    start_ai_gm_once(world);
    world.mark = table::player_mark(world);
    table::player_send(world, "travel docks_street", "\"kind\":\"queued\"");
    world.model.wait_past_script();
}

#[when(expr = "the GM's turn runs")]
fn the_gms_turn_runs(world: &mut World) {
    world.mark = table::player_mark(world);
    start_ai_gm_once(world);
    table::wait_rounds_to_view(world, world.mark);
}

#[when(expr = "the table stays quiet for longer than the graph's task timeout")]
fn the_table_stays_quiet(world: &mut World) {
    let gm = world.ai_gm.as_ref().expect("the AI GM runs");
    gm.advance(GRAPH_TASK_TIMEOUT + Duration::from_secs(1));
}

/// The tool results the model's last call carried, in order.
fn read_last_results(world: &World) -> Vec<String> {
    let calls = world.model.calls();
    read_tool_results(calls.last().expect("a model call"))
}

#[then(expr = "the table saw a perceive as Mira before any say")]
fn a_perceive_came_before_any_say(world: &mut World) {
    let results = read_last_results(world);
    let first = |needle: &str| results.iter().position(|result| result.contains(needle));
    let view = first("\"viewer\":\"mira\"").unwrap_or_else(|| panic!("{results:?}"));
    let say = first("\"kind\":\"say\"").unwrap_or_else(|| panic!("{results:?}"));
    assert!(view < say, "{results:?}");
}

#[then(expr = "the player saw Mira say hello")]
fn the_player_saw_mira_say_hello(world: &mut World) {
    table::wait_player_from(world, world.mark, "Mira: hello");
}

#[then(expr = "the model's next call carries the refusal as the tool result")]
fn the_next_call_carries_the_refusal(world: &mut World) {
    let calls = world.model.calls();
    assert!(
        matches!(calls[1].last(), Some(ChatMessage::ToolResult { content, .. }) if content.contains("\"kind\":\"refused\"")),
        "{:?}",
        calls[1].last()
    );
}

#[then(expr = "the scripted model queues wait for Mira and ends the turn")]
fn the_model_queues_wait_and_ends_the_turn(world: &mut World) {
    let results = read_last_results(world);
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(results[1].contains("\"kind\":\"queued\""), "{results:?}");
    assert!(results[1].contains("\"actor\":\"mira\""), "{results:?}");
    assert!(results[2].contains("\"kind\":\"committed\""), "{results:?}");
}

#[then(expr = "one round is committed")]
#[then(expr = "the client commits the round itself")]
fn one_round_is_committed(world: &mut World) {
    assert_eq!(table::wait_rounds_to_view(world, world.mark).len(), 1);
}

#[then(expr = "the round resolves the player's, Mira's and Aldric's moves together")]
fn the_round_resolves_three_moves(world: &mut World) {
    assert_eq!(table::wait_rounds_to_view(world, world.mark).len(), 1);
    let pack = table::pack(world);
    let key = list_round_keys(pack).pop().expect("a round");
    let text = read_record_line(pack, &key).unwrap_or_else(|err| panic!("{}", err.detail));
    let round: Value = serde_json::from_str(text.trim_end()).unwrap();
    let actors: Vec<&str> = round["intents"]
        .as_array()
        .expect("intents")
        .iter()
        .filter_map(|intent| intent["actor"].as_str()?.split(':').nth(1))
        .collect();
    for actor in ["rowan", "mira", "aldric"] {
        assert!(actors.contains(&actor), "{actors:?}");
    }
}

#[then(expr = "the player sees the GM narrate the result, not the dice")]
fn the_player_sees_the_narration(world: &mut World) {
    let seen = table::wait_player_from(world, world.mark, NARRATION);
    assert!(!seen.contains("faces"), "{seen}");
}

#[then(expr = "the client stops after {int} tool calls")]
fn the_client_stops_after(world: &mut World, count: usize) {
    assert_eq!(count, MAX_TOOL_CALLS);
    let calls = world.model.calls();
    assert_eq!(calls.len(), count);
    assert_eq!(read_tool_results(&calls[count - 1]).len(), count - 1);
}

#[then(expr = "the GM reports a step limit warning")]
fn the_gm_reports_a_step_limit_warning(world: &mut World) {
    let gm = world.ai_gm.as_ref().expect("the AI GM runs");
    table::wait_suffix(&gm.err, 0, "step limit");
}

#[then(expr = "the scripted model is called for that say")]
fn the_model_is_called_for_that_say(world: &mut World) {
    let calls = world.model.calls();
    assert_eq!(calls.len(), 1);
    let ChatMessage::Prompt { content } = &calls[0][0] else {
        panic!("{:?}", calls[0]);
    };
    assert!(content.contains("evening"), "{content}");
}

#[then(expr = "the AI GM is still connected")]
fn the_ai_gm_is_still_connected(world: &mut World) {
    let gm = world.ai_gm.as_ref().expect("the AI GM runs");
    assert!(
        !gm.thread.is_finished(),
        "{}",
        crate::support::text_of(&gm.err)
    );
}
