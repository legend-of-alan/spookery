//! The prose and the menu the model sees on one turn, and what it may answer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol::{
    self, CastKind, ClockView, ItemStack, MenuRow, NoteView, PackSummary, QueuedMove, Roll,
    SayArgs, SayEvent, Speaker, PERCEIVE_BODY_CHARS,
};

/// How many tape lines the prompt's `buffer` keeps.
const RECENT_LINES: usize = 16;

/// One tape line as the model reads it in the prompt's `buffer` and `lines`: a `SayEvent`
/// without `recipients`. The GM's own lines carry the `seq` the table answered with.
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

impl From<SayEvent> for TapeLine {
    fn from(say: SayEvent) -> Self {
        Self {
            seq: say.seq,
            from: say.from,
            address: say.address,
            place: say.place,
            actor: say.actor,
            text: say.text,
        }
    }
}

/// The model's answer to one prompt, the `act` arguments typed once: says and intents.
/// Read from the model's reply by `parse_completion` and fed to `Agent` as `Input::ModelReply`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelReply {
    pub says: Vec<SayArgs>,
    #[serde(default)]
    pub intents: Vec<protocol::IntentArgs>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneScope {
    Actor { actor: String },
    Place { place: String },
}

/// What starts a turn: a player's say, or a move in the queue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wake {
    Say(SayEvent),
    Queued,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnView {
    pub scope: SceneScope,
    pub player: Option<PlayerView>,
    pub npcs: Vec<NpcView>,
    pub place_body: String,
    pub cast_bodies: BTreeMap<String, String>,
}

/// A player's view as the prompt shows it: who and what is here, no facts and no notes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayerView {
    pub viewer: String,
    pub place: String,
    pub visible_actors: Vec<String>,
    pub clocks: Vec<ClockView>,
    pub items: Vec<ItemStack>,
    pub habits: Vec<String>,
}

/// An NPC's view as the prompt shows it: a player's, plus its facts and its notes as
/// `messages`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct NpcView {
    pub viewer: String,
    pub place: String,
    pub visible_actors: Vec<String>,
    pub facts: Vec<FactLine>,
    pub messages: Vec<NoteView>,
    pub clocks: Vec<ClockView>,
    pub items: Vec<ItemStack>,
    pub habits: Vec<String>,
}

/// A known fact as the prompt shows it. Never `owner_body`: a type rule, not a filter.
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
            clocks: view.clocks,
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
            clocks: view.clocks,
            items: view.items,
            habits: view.habits,
        }
    }
}

impl TurnView {
    /// The turn's views with the scene's place prose and the cast prose of everyone they
    /// show, each cut at the perceive budget.
    fn new(
        scope: SceneScope,
        player: Option<PlayerView>,
        npcs: Vec<NpcView>,
        pack: &PackSummary,
    ) -> Self {
        let place = match &scope {
            SceneScope::Place { place } => place.as_str(),
            SceneScope::Actor { .. } => player
                .as_ref()
                .map(|view| view.place.as_str())
                .filter(|place| !place.is_empty())
                .or_else(|| npcs.first().map(|view| view.place.as_str()))
                .unwrap_or_default(),
        };
        let place_body = pack.places.get(place).cloned().unwrap_or_default();
        let mut cast_bodies = BTreeMap::new();
        if let Some(view) = &player {
            include_cast(&mut cast_bodies, pack, &view.viewer, &view.visible_actors);
        }
        for view in &npcs {
            include_cast(&mut cast_bodies, pack, &view.viewer, &view.visible_actors);
        }
        Self {
            scope,
            player,
            npcs,
            place_body,
            cast_bodies,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedTurn {
    NoModel { says: Vec<SayArgs> },
    Model(Box<TurnView>),
}

/// Lines and queue that ride with the views. The prompt function does not take a canon.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptContext {
    pub buffer: Vec<TapeLine>,
    pub new_lines: Vec<TapeLine>,
    pub queue: Vec<QueuedMove>,
    pub checks: Vec<Roll>,
}

/// Add `line` to the prompt's `buffer`, which keeps the latest `RECENT_LINES`.
pub fn keep_recent(buffer: &mut Vec<TapeLine>, line: TapeLine) {
    buffer.push(line);
    if buffer.len() > RECENT_LINES {
        buffer.drain(..buffer.len() - RECENT_LINES);
    }
}

/// The says in `scope`, at most two, and the first intent on the menu.
pub fn keep_actions(
    reply: ModelReply,
    scope: &SceneScope,
    menu: &[MenuRow],
) -> (Vec<SayArgs>, Option<protocol::IntentArgs>) {
    let says = reply
        .says
        .into_iter()
        .filter(|say| is_say_in_scope(say, scope))
        .take(2)
        .collect();
    let intent = reply
        .intents
        .into_iter()
        .map(|mut call| {
            call.target = call.target.filter(|target| !target.is_empty());
            call
        })
        .find(|call| is_intent_on_menu(call, menu));
    (says, intent)
}

pub fn prepare_turn(
    pack: &PackSummary,
    wake: &Wake,
    present: &[String],
    perceive: &mut dyn FnMut(&str) -> Option<protocol::View>,
) -> PreparedTurn {
    let no_model = || PreparedTurn::NoModel { says: Vec::new() };
    let (speaker, place) = match wake {
        Wake::Say(SayEvent {
            from: Speaker::Player(speaker),
            address,
            actor,
            ..
        }) if address == "actor" => {
            let Some(actor) = actor.as_deref().filter(|id| !id.is_empty()) else {
                return absent_turn();
            };
            let Some(&kind) = pack.kinds.get(actor) else {
                return absent_turn();
            };
            let Some(view) = perceive(actor) else {
                return absent_turn();
            };
            if !view.visible_actors.contains(speaker) {
                return absent_turn();
            }
            let scope = SceneScope::Actor {
                actor: actor.to_string(),
            };
            return PreparedTurn::Model(Box::new(match kind {
                CastKind::Npc => TurnView::new(scope, None, vec![NpcView::from(view)], pack),
                CastKind::PcTemplate => {
                    TurnView::new(scope, Some(PlayerView::from(view)), Vec::new(), pack)
                }
            }));
        }
        Wake::Say(SayEvent {
            from: Speaker::Player(speaker),
            address,
            place,
            ..
        }) => {
            let named = place
                .as_ref()
                .filter(|id| address == "place" && !id.is_empty());
            (speaker.as_str(), named.cloned())
        }
        // The agent wakes only on a player's say.
        Wake::Say(_) => return no_model(),
        Wake::Queued => {
            let pcs: Vec<&String> = present
                .iter()
                .filter(|id| pack.kinds.get(id.as_str()) == Some(&CastKind::PcTemplate))
                .collect();
            let [speaker] = pcs[..] else {
                return no_model();
            };
            (speaker.as_str(), None)
        }
    };
    let Some(player) = perceive(speaker) else {
        return no_model();
    };
    let place = place.unwrap_or_else(|| player.place.clone());
    let npcs = npc_views(&player.visible_actors, &pack.kinds, &place, perceive);
    let scope = SceneScope::Place { place };
    PreparedTurn::Model(Box::new(TurnView::new(
        scope,
        Some(player.into()),
        npcs,
        pack,
    )))
}

pub fn render_prompt(
    menu: &[MenuRow],
    context: &PromptContext,
    view: &TurnView,
    checks: &[Roll],
) -> String {
    #[derive(Serialize)]
    struct CastBody<'a> {
        id: &'a str,
        body: &'a str,
    }
    #[derive(Serialize)]
    struct Views<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        player: Option<&'a PlayerView>,
        npcs: &'a [NpcView],
        place_body: &'a str,
        cast: Vec<CastBody<'a>>,
    }
    #[derive(Serialize)]
    struct Prompt<'a> {
        menu: &'a [MenuRow],
        buffer: &'a [TapeLine],
        lines: &'a [TapeLine],
        queue: &'a [QueuedMove],
        views: Views<'a>,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        checks: &'a [Roll],
    }
    let prompt = Prompt {
        menu,
        buffer: &context.buffer,
        lines: &context.new_lines,
        queue: &context.queue,
        views: Views {
            player: view.player.as_ref(),
            npcs: &view.npcs,
            place_body: &view.place_body,
            cast: view
                .cast_bodies
                .iter()
                .map(|(id, body)| CastBody { id, body })
                .collect(),
        },
        checks,
    };
    serde_json::to_string(&prompt).expect("prompt json")
}

fn absent_turn() -> PreparedTurn {
    PreparedTurn::NoModel {
        says: vec![SayArgs::Table {
            text: "They are not here.".to_string(),
        }],
    }
}

/// The views of the NPCs among `actors` who stand in `place`.
fn npc_views(
    actors: &[String],
    kinds: &BTreeMap<String, CastKind>,
    place: &str,
    perceive: &mut dyn FnMut(&str) -> Option<protocol::View>,
) -> Vec<NpcView> {
    actors
        .iter()
        .filter(|id| kinds.get(id.as_str()) == Some(&CastKind::Npc))
        .filter_map(|id| perceive(id))
        .filter(|view| view.place == place)
        .map(NpcView::from)
        .collect()
}

fn include_cast(
    bodies: &mut BTreeMap<String, String>,
    pack: &PackSummary,
    viewer: &str,
    actors: &[String],
) {
    for id in std::iter::once(viewer).chain(actors.iter().map(String::as_str)) {
        if let Some(body) = pack.cast.get(id) {
            let cut = body.chars().take(PERCEIVE_BODY_CHARS).collect();
            bodies.insert(id.to_string(), cut);
        }
    }
}

fn is_say_in_scope(say: &SayArgs, scope: &SceneScope) -> bool {
    match scope {
        SceneScope::Actor { actor } => {
            matches!(say, SayArgs::Actor { actor: id, .. } if id == actor)
        }
        SceneScope::Place { place } => match say {
            SayArgs::Table { .. } => true,
            SayArgs::Place { place: named, .. } => named == place,
            SayArgs::Actor { .. } => false,
        },
    }
}

/// Whether `call` names an actor and a move on the menu, with a target exactly when the
/// move takes one. A missing or empty actor or move is the one rule `protocol::IntentArgs` cannot
/// express.
fn is_intent_on_menu(call: &protocol::IntentArgs, menu: &[MenuRow]) -> bool {
    let named = |id: &Option<String>| id.as_deref().is_some_and(|id| !id.is_empty());
    let Some(row) = menu
        .iter()
        .find(|row| call.move_name.as_ref() == Some(&row.name))
    else {
        return false;
    };
    named(&call.actor_id)
        && named(&call.move_name)
        && call.target.is_some() == (row.target != "none")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn docks() -> PackSummary {
        let row = |n: &str, a: &str, t: &str| MenuRow {
            name: n.into(),
            kind: a.into(),
            target: t.into(),
        };
        PackSummary {
            menu: vec![
                row("wait", "verb", "none"),
                row("grit", "verb", "none"),
                row("travel", "verb", "exit"),
                row("restock_lamp_oil", "habit", "none"),
            ],
            kinds: BTreeMap::from([
                ("aldric".into(), CastKind::Npc),
                ("bram".into(), CastKind::Npc),
                ("mira".into(), CastKind::Npc),
                ("pc_rowan".into(), CastKind::PcTemplate),
            ]),
            places: BTreeMap::from([("docks_shop".into(), "The shop smells of tar.".into())]),
            cast: BTreeMap::from([
                ("aldric".into(), "Aldric keeps the counter.".into()),
                ("bram".into(), "Bram watches the pier.".into()),
                ("mira".into(), "Mira tends the lamp oil.".into()),
                ("pc_rowan".into(), "Rowan came in from the rain.".into()),
            ]),
        }
    }

    fn say(address: &str, actor: Option<&str>, text: &str) -> SayArgs {
        match address {
            "table" => SayArgs::Table {
                text: text.to_string(),
            },
            "actor" => SayArgs::Actor {
                actor: actor.unwrap().to_string(),
                text: text.to_string(),
            },
            "place" => SayArgs::Place {
                place: actor.unwrap().to_string(),
                text: text.to_string(),
            },
            _ => panic!("address"),
        }
    }

    fn intent(call: serde_json::Value) -> protocol::IntentArgs {
        serde_json::from_value(call).unwrap()
    }

    fn wake(address: &str, actor: Option<&str>) -> Wake {
        Wake::Say(SayEvent {
            seq: 1,
            from: Speaker::Player("pc_rowan".to_string()),
            address: address.to_string(),
            place: None,
            actor: actor.map(str::to_string),
            recipients: Vec::new(),
            text: "hello".to_string(),
        })
    }

    /// A view of `place` with `extra` fields, such as `known_facts` or `notes`, set.
    fn view_at(
        viewer: &str,
        place: &str,
        actors: &[&str],
        extra: serde_json::Value,
    ) -> protocol::View {
        let mut view = json!({
            "viewer": viewer,
            "place": place,
            "visible_actors": actors,
            "known_facts": [],
            "notes": [],
            "unknown_mentions": [],
            "clocks": [],
            "items": [],
            "habits": []
        });
        for (key, value) in extra.as_object().unwrap() {
            view[key] = value.clone();
        }
        serde_json::from_value(view).unwrap()
    }

    fn shop(viewer: &str, actors: &[&str]) -> protocol::View {
        view_at(viewer, "docks_shop", actors, json!({}))
    }

    fn prompt_for(
        pack: &PackSummary,
        mut perceive: impl FnMut(&str) -> Option<protocol::View>,
    ) -> String {
        let prepared = prepare_turn(pack, &wake("table", None), &[], &mut perceive);
        let PreparedTurn::Model(view) = prepared else {
            panic!("model was not called");
        };
        render_prompt(&pack.menu, &PromptContext::default(), &view, &[])
    }

    /// Feature: The AI GM gets non-hidden prose
    ///
    /// Scenario: Cast prose reaches the prompt
    ///   Then the prompt contains mira's cast body
    #[test]
    fn cast_prose_reaches_the_prompt() {
        let mut pack = docks();
        let mira = pack.cast["mira"].trim().to_string();
        let place = pack.places["docks_shop"].trim().to_string();
        pack.cast
            .insert("bram".to_string(), "BRAM-ABSENT".to_string());
        assert!(!mira.is_empty(), "mira has cast prose");
        assert!(!place.is_empty(), "the shop has place prose");
        let prompt = prompt_for(&pack, |viewer| {
            Some(shop(viewer, &["aldric", "mira", "pc_rowan"]))
        });
        assert!(prompt.contains(&mira), "{prompt}");
        assert!(prompt.contains(&place), "{prompt}");
        assert!(!prompt.contains("BRAM-ABSENT"), "{prompt}");
    }

    #[test]
    fn a_long_cast_body_is_cut_at_the_budget() {
        let mut pack = docks();
        let budget = PERCEIVE_BODY_CHARS;
        pack.cast
            .insert("aldric".to_string(), "Z".repeat(budget + 1));
        let prompt = prompt_for(&pack, |viewer| Some(shop(viewer, &["aldric", "pc_rowan"])));
        assert!(prompt.contains(&"Z".repeat(budget)), "{prompt}");
        assert!(!prompt.contains(&"Z".repeat(budget + 1)), "{prompt}");
    }

    /// Scenario: Hidden facts never do
    ///   Given a hidden fact body "BODY-SENTINEL"
    ///   Then the prompt does not contain "BODY-SENTINEL"
    #[test]
    fn prompt_omits_dropped_strings_and_keeps_fact_text() {
        let mut pack = docks();
        pack.cast
            .insert("mira".to_string(), "MIRA-PROSE-SENTINEL".to_string());
        let actors = ["aldric", "mira", "pc_rowan"];
        let mut calls = 0;
        let prompt = prompt_for(&pack, |viewer| {
            calls += 1;
            let extra = match viewer {
                "pc_rowan" => json!({
                    "known_facts": [{"id": "k", "text": "KNOWN-SENTINEL", "owner_body": "KNOWN-BODY"}],
                    "notes": [{"id": "m", "to": [], "fact_ids": [], "text": "MESSAGE-SENTINEL"}],
                    "unknown_mentions": ["UNKNOWN-SENTINEL"]
                }),
                "aldric" => json!({
                    "known_facts": [{"id": "f", "text": "FACT-TEXT-SENTINEL", "owner_body": "BODY-SENTINEL"}]
                }),
                _ => return None,
            };
            Some(view_at(viewer, "docks_shop", &actors, extra))
        });
        assert!(!prompt.contains("KNOWN-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("KNOWN-BODY"), "{prompt}");
        assert!(!prompt.contains("BODY-SENTINEL"), "{prompt}");
        assert!(prompt.contains("MIRA-PROSE-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("MESSAGE-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("UNKNOWN-SENTINEL"), "{prompt}");
        assert!(prompt.contains("FACT-TEXT-SENTINEL"), "{prompt}");
        assert!(calls >= 2, "player and npc views");
    }

    #[test]
    fn address_rule_runs_before_the_cap_and_a_bad_intent_keeps_the_says() {
        let reply = ModelReply {
            says: vec![
                say("table", None, "rejected"),
                say("actor", Some("mira"), "one"),
                say("actor", Some("mira"), "two"),
                say("actor", Some("mira"), "three"),
            ],
            intents: vec![
                intent(json!({"move": "wait"})),
                intent(json!({"actor_id": "mira", "move": "not_a_move"})),
                intent(json!({"actor_id": "mira", "move": "wait", "target": ""})),
                intent(json!({"actor_id": "mira", "move": "grit"})),
            ],
        };
        let scope = SceneScope::Actor {
            actor: "mira".to_string(),
        };
        let (says, intent) = keep_actions(reply, &scope, &docks().menu);
        assert_eq!(
            says.iter().map(SayArgs::text).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        let intent = intent.unwrap();
        assert_eq!(intent.move_name.as_deref(), Some("wait"));
        assert_eq!(intent.target, None);
    }

    #[test]
    fn queued_wake_keeps_a_table_say_and_drops_an_actor_say() {
        let reply = ModelReply {
            says: vec![
                say("table", None, "here"),
                say("actor", Some("mira"), "aside"),
            ],
            intents: vec![],
        };
        let scope = SceneScope::Place {
            place: "docks_shop".to_string(),
        };
        let (says, _) = keep_actions(reply, &scope, &docks().menu);
        assert_eq!(says.len(), 1);
        assert_eq!(says[0].text(), "here");
    }

    #[test]
    fn sit_with_no_pc_skips_the_model() {
        let pack = docks();
        let mut called = false;
        let prepared = prepare_turn(&pack, &Wake::Queued, &[], &mut |_| {
            called = true;
            None
        });
        assert_eq!(prepared, PreparedTurn::NoModel { says: Vec::new() });
        assert!(!called);
    }

    #[test]
    fn different_places_say_they_are_not_here() {
        let pack = docks();
        let mut calls = Vec::new();
        let prepared = prepare_turn(&pack, &wake("actor", Some("mira")), &[], &mut |viewer| {
            calls.push(viewer.to_string());
            Some(view_at(viewer, "mira_rooms", &["mira"], json!({})))
        });
        assert_eq!(prepared, absent_turn());
        assert_eq!(calls, vec!["mira".to_string()]);
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
