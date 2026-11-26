//! The AI GM's turn loop against a real host: perceive first, carry the pack's gm_notes,
//! recover from a refusal, queue several NPCs, stop at the model call limit, and outwait
//! the graph's task timeout.

use spookery_engine::component;
use std::time::Duration;

use cucumber::{given, then, when};
use serde_json::Value;
use spookery::agent::{model, MAX_MODEL_CALLS};
use spookery_engine::testing::{act_of, world_of};

use crate::support::copy_pack;
use crate::World;

use super::host;
use super::scripted_gm::{read_tool_results, start_ai_gm};

/// graph-flow's default task timeout, which the turn graph keeps.
const GRAPH_TASK_TIMEOUT: Duration = Duration::from_secs(300);

const END_TURN: (&str, &str) = ("end_turn", "{}");
const QUEUE_MIRA_WAIT: (&str, &str) = ("queue", r#"{"actor":"mira","verb":"wait"}"#);
const NARRATION: &str = "The street opens past the shop door.";

#[given(expr = "a docks table with a seated player and the NPC Mira in the shop")]
#[given(expr = "a docks table with a seated player and the NPCs Mira and Aldric in the shop")]
async fn a_docks_table_with_npcs_in_the_shop(world: &mut World) {
    start_docks_table_with_mira(world, None).await;
}

#[given(
    expr = "a docks table with a seated player, the NPC Mira in the shop, and gm_notes {string}"
)]
async fn a_docks_table_with_mira_and_gm_notes(world: &mut World, notes: String) {
    start_docks_table_with_mira(world, Some(&notes)).await;
}

/// Copies the docks pack, writes `notes` into `pack.yaml` when some are given, moves Mira
/// to the shop, and seats a player. The notes have to be on disk before the host starts.
async fn start_docks_table_with_mira(world: &mut World, notes: Option<&str>) {
    let pack = copy_pack("docks", "ai-gm-loop");
    if let Some(notes) = notes {
        host::append_gm_notes(&pack, notes);
    }
    super::common::edit_save(&pack, |now| {
        now.actors_mut()
            .get_mut::<component::Place>(&"mira".into())
            .set("docks_shop")
            .unwrap();
    });
    let ecs_world = world_of(&pack);
    for npc in ["mira", "aldric"] {
        let npc = npc.into();
        let place = ecs_world.actors().get::<component::Place>(&npc).place_id();
        assert_eq!(
            place.map(|place| place.as_str()),
            Some("docks_shop"),
            "{npc}"
        );
    }
    host::listen(world, &pack).await;
    host::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a seated player who has queued a move")]
async fn a_docks_table_with_a_queued_verb(world: &mut World) {
    let (pack, _) = super::ai_gm::queued_travel(world, "ai-gm-stuck").await;
    world.pack = Some(pack);
}

#[given(expr = "a docks table with a seated player and the AI GM connected")]
async fn a_docks_table_with_the_ai_gm_sitting(world: &mut World) {
    let pack = copy_pack("docks", "ai-gm-quiet");
    host::listen(world, &pack).await;
    host::seat_rowan(world, &pack).await;
    world.pack = Some(pack);
    start_ai_gm_once(world).await;
}

#[given(
    expr = "a scripted model that perceives as Mira, then has Mira say hello, then ends the turn"
)]
fn a_model_that_perceives_first(world: &mut World) {
    world.model.push(&[("perceive", r#"{"as":"mira"}"#)]);
    world.model.push(&[(
        "say",
        r#"{"to":{"place":"docks_shop"},"text":"hello","as":"mira"}"#,
    )]);
    world.model.push(&[END_TURN]);
}

#[given(expr = "a scripted model that perceives as Mira and as Rowan, then ends the turn")]
fn a_model_that_perceives_mira_and_rowan(world: &mut World) {
    world.model.push(&[
        ("perceive", r#"{"as":"mira"}"#),
        ("perceive", r#"{"as":"rowan"}"#),
    ]);
    world.model.push(&[END_TURN]);
}

#[given(expr = "a scripted model that queues a move Mira can't make")]
fn a_model_that_queues_a_bad_verb(world: &mut World) {
    world
        .model
        .push(&[("queue", r#"{"actor":"mira","verb":"fly"}"#)]);
    world.model.push(&[QUEUE_MIRA_WAIT]);
    world.model.push(&[END_TURN]);
}

#[given(
    expr = "a scripted model that queues a move for Mira and a move for Aldric, then ends the turn"
)]
fn a_model_that_queues_two_npcs(world: &mut World) {
    world.model.push(&[
        QUEUE_MIRA_WAIT,
        ("queue", r#"{"actor":"aldric","verb":"wait"}"#),
    ]);
    world.model.push(&[END_TURN]);
    let say = format!(r#"{{"to":"table","text":"{NARRATION}"}}"#);
    world.model.push(&[("say", &say)]);
}

#[given(expr = "a scripted model that only ever calls perceive")]
fn a_model_that_only_perceives(world: &mut World) {
    world.model.repeat(&[("perceive", r#"{"as":"rowan"}"#)]);
}

/// Starts the AI GM with the scenario's model, unless it already runs.
async fn start_ai_gm_once(world: &mut World) {
    if world.ai_gm.is_none() {
        world.ai_gm = Some(start_ai_gm(world, world.model.clone()).await);
    }
}

#[when(expr = "the player says {string} in the shop")]
async fn the_player_says(world: &mut World, text: String) {
    player_says(world, &text).await;
}

/// The player's "say" starts the turn whose queue the host refuses.
#[when(expr = "the table refuses the queue")]
async fn the_table_refuses_the_queue(world: &mut World) {
    player_says(world, "evening").await;
}

/// Starts the AI GM, has the player say `text`, and waits until the turn it started is done.
async fn player_says(world: &mut World, text: &str) {
    start_ai_gm_once(world).await;
    world.mark = host::player_mark(world);
    host::player_send(world, &format!("say {text}"), "\"kind\":\"say\"").await;
    world.model.wait_past_script().await;
}

#[when(expr = "the player queues travel")]
async fn the_player_queues_travel(world: &mut World) {
    start_ai_gm_once(world).await;
    world.mark = host::player_mark(world);
    host::player_send(world, "travel docks_street", "\"kind\":\"queued\"").await;
    world.model.wait_past_script().await;
}

#[when(expr = "the GM's turn runs")]
async fn the_gms_turn_runs(world: &mut World) {
    world.mark = host::player_mark(world);
    start_ai_gm_once(world).await;
    host::wait_rounds_to_view(world, world.mark).await;
}

#[when(expr = "the table stays quiet for longer than the graph's task timeout")]
async fn the_table_stays_quiet(world: &mut World) {
    let gm = world.ai_gm.as_ref().expect("the AI GM runs");
    gm.advance(GRAPH_TASK_TIMEOUT + Duration::from_secs(1))
        .await;
}

/// The tool results the model's last call carried, in order.
fn read_last_results(world: &World) -> Vec<String> {
    let calls = world.model.calls();
    read_tool_results(calls.last().expect("a model call"))
}

#[then(expr = "every model call carries the pack's gm_notes {string}")]
fn every_model_call_carries_the_pack_gm_notes(world: &mut World, notes: String) {
    let calls = world.model.calls();
    assert!(!calls.is_empty(), "{calls:?}");
    for messages in &calls {
        let shown = messages.iter().any(
            |message| matches!(message, model::Message::System { content } if content.contains(&notes)),
        );
        assert!(shown, "{calls:?}");
    }
}

#[then(expr = "the table saw a perceive as Mira before any say")]
fn a_perceive_came_before_any_say(world: &mut World) {
    let results = read_last_results(world);
    let first = |needle: &str| results.iter().position(|result| result.contains(needle));
    let view = first("\"viewer\":\"mira\"").unwrap_or_else(|| panic!("{results:?}"));
    let say = first("\"kind\":\"say\"").unwrap_or_else(|| panic!("{results:?}"));
    assert!(view < say, "{results:?}");
}

/// The perceive result the model read for `viewer`, on its last call.
fn read_perceive_result(world: &World, viewer: &str) -> Value {
    let results = read_last_results(world);
    results
        .iter()
        .filter_map(|result| serde_json::from_str::<Value>(result).ok())
        .find(|result| result["viewer"] == viewer)
        .unwrap_or_else(|| panic!("no perceive as {viewer}\n{results:?}"))
}

/// The shop's prose and the prose of each actor in it, and none for Bram in the street.
fn assert_shop_prose(result: &Value) {
    let shop = result["place_body"].as_str().unwrap_or_default();
    assert!(shop.starts_with("The shop faces the water."), "{result}");
    for (id, prose) in [
        ("aldric", "Aldric sweeps the shop"),
        ("mira", "Mira keeps the shop neat"),
        ("rowan", "Rowan keeps the shop"),
    ] {
        let shown = result["cast"][id].as_str().unwrap_or_default();
        assert!(shown.starts_with(prose), "{result}");
    }
    assert!(result["cast"].get("bram").is_none(), "{result}");
}

#[then(expr = "the perceive as Mira holds the cast's prose and Mira's notes")]
fn miras_perceive_holds_her_notes(world: &mut World) {
    let result = read_perceive_result(world, "mira");
    assert_shop_prose(&result);
    let notes = result["notes"]
        .as_array()
        .unwrap_or_else(|| panic!("{result}"));
    let texts: Vec<&str> = notes
        .iter()
        .filter_map(|note| note["text"].as_str())
        .collect();
    assert!(texts.contains(&"The lamp-oil ledger is short."), "{result}");
}

#[then(expr = "the perceive as Rowan holds the cast's prose but no notes")]
fn rowans_perceive_holds_no_notes(world: &mut World) {
    let result = read_perceive_result(world, "rowan");
    assert_shop_prose(&result);
    assert!(result.get("notes").is_none(), "{result}");
}

#[then(expr = "the player saw Mira say hello")]
async fn the_player_saw_mira_say_hello(world: &mut World) {
    super::speech::wait_for_line(world, "rowan", world.mark, "Mira: hello").await;
}

#[then(expr = "the model's next call shows Mira's \"hello\" from npc:mira")]
async fn the_next_call_shows_miras_hello(world: &mut World) {
    host::poll_until(
        "Mira's hello from npc:mira on the model's next call",
        async || {
            let calls = world.model.calls();
            let Some(call) = calls.last() else {
                return Err("no model call".to_string());
            };
            let Some(content) = call.iter().find_map(|message| match message {
                model::Message::User { content } => Some(content),
                _ => None,
            }) else {
                return Err(format!("{call:?}"));
            };
            if content.contains("npc:mira") && content.contains("hello") {
                Ok(())
            } else {
                Err(content.clone())
            }
        },
    )
    .await;
}

#[then(expr = "the model's next call carries the refusal as the tool result")]
fn the_next_call_carries_the_refusal(world: &mut World) {
    let calls = world.model.calls();
    assert!(
        matches!(calls[1].last(), Some(model::Message::Tool { content, .. }) if content.contains("\"kind\":\"refused\"")),
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
async fn one_round_is_committed(world: &mut World) {
    assert_eq!(host::wait_rounds_to_view(world, world.mark).await.len(), 1);
}

#[then(expr = "the round resolves the player's, Mira's and Aldric's moves together")]
async fn the_round_resolves_three_verbs(world: &mut World) {
    assert_eq!(host::wait_rounds_to_view(world, world.mark).await.len(), 1);
    let now = world_of(host::pack(world));
    for actor in ["rowan", "mira", "aldric"] {
        assert_eq!(
            act_of(&now, actor).last_act_round,
            now.round_index,
            "{actor}"
        );
    }
}

#[then(expr = "the player sees the GM narrate the result, not the dice")]
async fn the_player_sees_the_narration(world: &mut World) {
    let seen = host::wait_player_from(world, world.mark, NARRATION).await;
    assert!(!seen.contains("\"dice\""), "{seen}");
}

#[then(expr = "the client stops after {int} model calls")]
fn the_client_stops_after(world: &mut World, count: usize) {
    assert_eq!(count, MAX_MODEL_CALLS);
    let calls = world.model.calls();
    assert_eq!(calls.len(), count);
    assert_eq!(read_tool_results(&calls[count - 1]).len(), count - 1);
}

#[then(expr = "the GM reports a model call limit warning")]
async fn the_gm_reports_a_model_call_limit_warning(world: &mut World) {
    let gm = world.ai_gm.as_ref().expect("the AI GM runs");
    host::wait_suffix(&gm.err, 0, "model call limit").await;
}

#[then(expr = "the scripted model is called for that say")]
fn the_model_is_called_for_that_say(world: &mut World) {
    let calls = world.model.calls();
    let model::Message::User { content } = &calls[0][1] else {
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
