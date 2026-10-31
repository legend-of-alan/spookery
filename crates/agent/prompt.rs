//! The prose and the menu the model sees on one turn.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use crate::protocol::{CastKind, MenuRow, PackSummary, SayArgs};
use crate::protocol::{QueuedMove, Roll, Speaker, PERCEIVE_BODY_CHARS};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct HandledSay {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    pub from: Speaker,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
    pub text: String,
}

impl HandledSay {
    pub fn from_outgoing(say: &SayArgs, seq: Option<u64>) -> Self {
        match say {
            SayArgs::Table { text } => Self {
                seq,
                from: Speaker::Gm,
                address: "table".to_string(),
                place: None,
                actor: None,
                text: text.clone(),
            },
            SayArgs::Place { place, text } => Self {
                seq,
                from: Speaker::Gm,
                address: "place".to_string(),
                place: Some(place.clone()),
                actor: None,
                text: text.clone(),
            },
            SayArgs::Actor { actor, text } => Self {
                seq,
                from: Speaker::Gm,
                address: "actor".to_string(),
                place: None,
                actor: Some(actor.clone()),
                text: text.clone(),
            },
        }
    }
}

impl From<crate::protocol::SayEvent> for HandledSay {
    fn from(say: crate::protocol::SayEvent) -> Self {
        Self {
            seq: Some(say.seq),
            from: say.from,
            address: say.address,
            place: say.place,
            actor: say.actor,
            text: say.text,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    pub actor_id: String,
    pub move_name: String,
    pub target: Option<String>,
}

impl Intent {
    pub fn to_value(&self) -> Value {
        let mut body = serde_json::json!({
            "actor_id": self.actor_id,
            "move": self.move_name,
        });
        if let Some(target) = &self.target {
            body["target"] = serde_json::json!(target);
        }
        body
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelActions {
    pub says: Vec<SayArgs>,
    pub intents: Vec<Intent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneScope {
    Actor { actor: String },
    Place { place: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeptActions {
    pub says: Vec<SayArgs>,
    pub intent: Option<Intent>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wake {
    Say {
        from: Speaker,
        seq: u64,
        address: String,
        place: Option<String>,
        actor: Option<String>,
        text: String,
    },
    Queued,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnView {
    pub scope: SceneScope,
    pub player: Option<Value>,
    pub npcs: Vec<Value>,
    pub place_body: String,
    pub cast_bodies: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreparedTurn {
    NoModel { says: Vec<SayArgs> },
    Model(TurnView),
}

/// Lines and queue that ride with the views. The prompt function does not take a canon.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptContext {
    pub buffer: Vec<HandledSay>,
    pub new_lines: Vec<HandledSay>,
    pub queue: Vec<QueuedMove>,
    pub checks: Vec<Roll>,
}

pub fn push_handled(buffer: &mut Vec<HandledSay>, say: HandledSay) {
    buffer.push(say);
    if buffer.len() > 16 {
        let extra = buffer.len() - 16;
        buffer.drain(0..extra);
    }
}

/// `None` means the JSON is not the action object. That is an empty action list.
pub fn parse_actions(text: &str, says_only: bool) -> Option<ModelActions> {
    if says_only {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct SaysOnly {
            says: Vec<SayArgs>,
        }
        let parsed: SaysOnly = serde_json::from_str(text).ok()?;
        return Some(ModelActions {
            says: parsed.says,
            intents: Vec::new(),
        });
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FullActions {
        says: Vec<SayArgs>,
        intents: Vec<IntentJson>,
    }
    let parsed: FullActions = serde_json::from_str(text).ok()?;
    let intents = parsed
        .intents
        .into_iter()
        .map(IntentJson::into_intent)
        .collect::<Option<Vec<_>>>()?;
    Some(ModelActions {
        says: parsed.says,
        intents,
    })
}

pub fn keep_actions(actions: ModelActions, scope: &SceneScope, menu: &[MenuRow]) -> KeptActions {
    let mut says = Vec::new();
    for say in actions.says {
        if !is_say_in_scope(&say, scope) {
            continue;
        }
        if says.len() == 2 {
            continue;
        }
        says.push(say);
    }
    let mut intent = None;
    for item in actions.intents {
        if !is_intent_on_menu(&item, menu) {
            continue;
        }
        if intent.is_some() {
            continue;
        }
        intent = Some(item);
    }
    KeptActions { says, intent }
}

pub fn prepare_turn(
    pack: &PackSummary,
    wake: &Wake,
    present: &[String],
    perceive: &mut dyn FnMut(&str) -> Result<Value, String>,
) -> PreparedTurn {
    match wake {
        Wake::Say {
            from,
            address,
            actor,
            ..
        } if address == "actor" => {
            let Some(actor) = actor.as_deref().filter(|id| !id.is_empty()) else {
                return absent_turn();
            };
            let speaker = speaker_id(from);
            let Some(&kind) = pack.kinds.get(actor) else {
                return absent_turn();
            };
            let Ok(view) = perceive(actor) else {
                return absent_turn();
            };
            if !has_visible_actor(&view, speaker) {
                return absent_turn();
            }
            match kind {
                CastKind::Npc => PreparedTurn::Model(build_view(
                    SceneScope::Actor {
                        actor: actor.to_string(),
                    },
                    None,
                    vec![trim_npc_view(&view)],
                    pack,
                )),
                CastKind::PcTemplate => PreparedTurn::Model(build_view(
                    SceneScope::Actor {
                        actor: actor.to_string(),
                    },
                    Some(trim_player_view(&view)),
                    Vec::new(),
                    pack,
                )),
            }
        }
        Wake::Say {
            from,
            address,
            place,
            ..
        } => {
            let speaker = speaker_id(from);
            let Ok(player) = perceive(speaker) else {
                return PreparedTurn::NoModel { says: Vec::new() };
            };
            let speaker_place = place_of(&player);
            let filter = if address == "place" {
                place
                    .clone()
                    .filter(|id| !id.is_empty())
                    .unwrap_or(speaker_place)
            } else {
                speaker_place
            };
            let npcs = npc_views(&player, &pack.kinds, &filter, perceive);
            PreparedTurn::Model(build_view(
                SceneScope::Place { place: filter },
                Some(trim_player_view(&player)),
                npcs,
                pack,
            ))
        }
        Wake::Queued => {
            let pcs: Vec<&str> = present
                .iter()
                .filter(|id| pack.kinds.get(id.as_str()) == Some(&CastKind::PcTemplate))
                .map(String::as_str)
                .collect();
            if pcs.len() != 1 {
                return PreparedTurn::NoModel { says: Vec::new() };
            }
            let speaker = pcs[0];
            let Ok(player) = perceive(speaker) else {
                return PreparedTurn::NoModel { says: Vec::new() };
            };
            let place = place_of(&player);
            let npcs = npc_views(&player, &pack.kinds, &place, perceive);
            PreparedTurn::Model(build_view(
                SceneScope::Place { place },
                Some(trim_player_view(&player)),
                npcs,
                pack,
            ))
        }
    }
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
        player: Option<&'a Value>,
        npcs: &'a [Value],
        place_body: &'a str,
        cast: Vec<CastBody<'a>>,
    }
    #[derive(Serialize)]
    struct Prompt<'a> {
        menu: &'a [MenuRow],
        buffer: &'a [HandledSay],
        lines: &'a [HandledSay],
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

fn npc_views(
    player: &Value,
    kinds: &BTreeMap<String, CastKind>,
    place: &str,
    perceive: &mut dyn FnMut(&str) -> Result<Value, String>,
) -> Vec<Value> {
    let Some(ids) = player.get("visible_actors").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in ids {
        let Some(id) = id.as_str() else {
            continue;
        };
        if kinds.get(id) != Some(&CastKind::Npc) {
            continue;
        }
        let Ok(view) = perceive(id) else {
            continue;
        };
        if view.get("place").and_then(Value::as_str) != Some(place) {
            continue;
        }
        out.push(trim_npc_view(&view));
    }
    out
}

fn trim_player_view(view: &Value) -> Value {
    serde_json::json!({
        "viewer": view.get("viewer").cloned().unwrap_or(Value::Null),
        "place": view.get("place").cloned().unwrap_or(Value::Null),
        "visible_actors": view.get("visible_actors").cloned().unwrap_or(Value::Array(Vec::new())),
        "clocks": view.get("clocks").cloned().unwrap_or(Value::Array(Vec::new())),
        "items": view.get("items").cloned().unwrap_or(Value::Array(Vec::new())),
        "habits": view.get("habits").cloned().unwrap_or(Value::Array(Vec::new())),
    })
}

fn trim_npc_view(view: &Value) -> Value {
    let facts = view
        .get("known_facts")
        .and_then(Value::as_array)
        .map(|facts| {
            facts
                .iter()
                .map(|fact| {
                    serde_json::json!({
                        "id": fact.get("id").cloned().unwrap_or(Value::Null),
                        "text": fact.get("text").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    serde_json::json!({
        "viewer": view.get("viewer").cloned().unwrap_or(Value::Null),
        "place": view.get("place").cloned().unwrap_or(Value::Null),
        "visible_actors": view.get("visible_actors").cloned().unwrap_or(Value::Array(Vec::new())),
        "facts": facts,
        "messages": view.get("notes").cloned().unwrap_or(Value::Array(Vec::new())),
        "clocks": view.get("clocks").cloned().unwrap_or(Value::Array(Vec::new())),
        "items": view.get("items").cloned().unwrap_or(Value::Array(Vec::new())),
        "habits": view.get("habits").cloned().unwrap_or(Value::Array(Vec::new())),
    })
}

fn build_view(
    scope: SceneScope,
    player: Option<Value>,
    npcs: Vec<Value>,
    pack: &PackSummary,
) -> TurnView {
    let place = scene_place(&scope, &player, &npcs);
    let place_body = pack.places.get(&place).cloned().unwrap_or_default();
    let mut cast_bodies = BTreeMap::new();
    if let Some(view) = &player {
        include_view(&mut cast_bodies, pack, view);
    }
    for view in &npcs {
        include_view(&mut cast_bodies, pack, view);
    }
    TurnView {
        scope,
        player,
        npcs,
        place_body,
        cast_bodies,
    }
}

fn scene_place(scope: &SceneScope, player: &Option<Value>, npcs: &[Value]) -> String {
    match scope {
        SceneScope::Place { place } => place.clone(),
        SceneScope::Actor { .. } => player
            .as_ref()
            .map(place_of)
            .filter(|place| !place.is_empty())
            .or_else(|| npcs.first().map(place_of))
            .unwrap_or_default(),
    }
}

fn include_view(bodies: &mut BTreeMap<String, String>, pack: &PackSummary, view: &Value) {
    if let Some(viewer) = view.get("viewer").and_then(Value::as_str) {
        include_cast(bodies, pack, viewer);
    }
    let Some(actors) = view.get("visible_actors").and_then(Value::as_array) else {
        return;
    };
    for actor in actors {
        if let Some(id) = actor.as_str() {
            include_cast(bodies, pack, id);
        }
    }
}

fn include_cast(bodies: &mut BTreeMap<String, String>, pack: &PackSummary, id: &str) {
    if let Some(body) = pack.cast.get(id) {
        let cut = body.chars().take(PERCEIVE_BODY_CHARS).collect();
        bodies.insert(id.to_string(), cut);
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

fn is_intent_on_menu(intent: &Intent, menu: &[MenuRow]) -> bool {
    let Some(row) = menu.iter().find(|row| row.name == intent.move_name) else {
        return false;
    };
    if row.target == "none" {
        intent.target.is_none()
    } else {
        intent.target.is_some()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IntentJson {
    actor_id: String,
    #[serde(rename = "move")]
    move_name: String,
    #[serde(default)]
    target: Option<String>,
}

impl IntentJson {
    fn into_intent(self) -> Option<Intent> {
        if self.actor_id.is_empty() || self.move_name.is_empty() {
            return None;
        }
        let target = self.target.filter(|t| !t.is_empty());
        Some(Intent {
            actor_id: self.actor_id,
            move_name: self.move_name,
            target,
        })
    }
}

fn has_visible_actor(view: &Value, id: &str) -> bool {
    view.get("visible_actors")
        .and_then(Value::as_array)
        .is_some_and(|ids| ids.iter().any(|item| item.as_str() == Some(id)))
}

fn place_of(view: &Value) -> String {
    view.get("place")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn speaker_id(from: &Speaker) -> &str {
    match from {
        Speaker::Player(id) => id,
        Speaker::Gm | Speaker::Npc(_) => "",
    }
}

#[cfg(test)]
mod tests {
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

    fn intent(move_name: &str, target: Option<&str>) -> Intent {
        Intent {
            actor_id: "mira".to_string(),
            move_name: move_name.to_string(),
            target: target.map(str::to_string),
        }
    }

    fn table_wake() -> Wake {
        Wake::Say {
            from: Speaker::Player("pc_rowan".to_string()),
            seq: 1,
            address: "table".to_string(),
            place: None,
            actor: None,
            text: "hello".to_string(),
        }
    }

    fn shop(viewer: &str, actors: &[&str]) -> Value {
        serde_json::json!({
            "viewer": viewer,
            "place": "docks_shop",
            "visible_actors": actors,
            "known_facts": [],
            "notes": [],
            "unknown_mentions": [],
            "clocks": [],
            "items": [],
            "habits": []
        })
    }

    fn prompt_for(
        pack: &PackSummary,
        mut perceive: impl FnMut(&str) -> Result<Value, String>,
    ) -> String {
        let prepared = prepare_turn(pack, &table_wake(), &[], &mut perceive);
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
            Ok(shop(viewer, &["aldric", "mira", "pc_rowan"]))
        });
        assert!(prompt.contains(&mira), "{prompt}");
        assert!(prompt.contains(&place), "{prompt}");
        assert!(!prompt.contains("BRAM-ABSENT"), "{prompt}");
    }

    /// Scenario: Hidden facts never do
    ///   Given a hidden fact body "BODY-SENTINEL"
    ///   Then the prompt does not contain "BODY-SENTINEL"
    #[test]
    fn hidden_facts_never_reach_the_prompt() {
        let pack = docks();
        let prompt = prompt_for(&pack, |viewer| {
            let mut view = shop(viewer, &["aldric", "mira", "pc_rowan"]);
            view["known_facts"] = serde_json::json!([
                {"id": "fact_ledger_short", "text": "The lamp-oil ledger is short.", "owner_body": "BODY-SENTINEL"}
            ]);
            Ok(view)
        });
        assert!(!prompt.contains("BODY-SENTINEL"), "{prompt}");
    }

    #[test]
    fn a_long_cast_body_is_cut_at_the_budget() {
        let mut pack = docks();
        let budget = PERCEIVE_BODY_CHARS;
        pack.cast
            .insert("aldric".to_string(), "Z".repeat(budget + 1));
        let prompt = prompt_for(&pack, |viewer| Ok(shop(viewer, &["aldric", "pc_rowan"])));
        assert!(prompt.contains(&"Z".repeat(budget)), "{prompt}");
        assert!(!prompt.contains(&"Z".repeat(budget + 1)), "{prompt}");
    }

    #[test]
    fn prompt_omits_dropped_strings_and_keeps_fact_text() {
        let mut pack = docks();
        pack.cast
            .insert("mira".to_string(), "MIRA-PROSE-SENTINEL".to_string());
        let mut calls = 0;
        let prepared = prepare_turn(&pack, &table_wake(), &[], &mut |viewer| {
            calls += 1;
            if viewer == "pc_rowan" {
                return Ok(serde_json::json!({
                    "viewer": "pc_rowan",
                    "place": "docks_shop",
                    "visible_actors": ["aldric", "mira", "pc_rowan"],
                    "known_facts": [{"id": "k", "text": "KNOWN-SENTINEL", "owner_body": "KNOWN-BODY"}],
                    "notes": [{"id": "m", "text": "MESSAGE-SENTINEL"}],
                    "unknown_mentions": ["UNKNOWN-SENTINEL"],
                    "clocks": [],
                    "items": [],
                    "habits": []
                }));
            }
            if viewer == "aldric" {
                return Ok(serde_json::json!({
                    "viewer": "aldric",
                    "place": "docks_shop",
                    "visible_actors": ["aldric", "mira", "pc_rowan"],
                    "known_facts": [{"id": "f", "text": "FACT-TEXT-SENTINEL", "owner_body": "BODY-SENTINEL"}],
                    "notes": [],
                    "unknown_mentions": [],
                    "clocks": [],
                    "items": [],
                    "habits": []
                }));
            }
            Err("no".into())
        });
        let PreparedTurn::Model(view) = prepared else {
            panic!("model was not called");
        };
        let prompt = render_prompt(&pack.menu, &PromptContext::default(), &view, &[]);
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
    fn address_rule_runs_before_the_cap() {
        let menu = docks().menu;
        let actions = ModelActions {
            says: vec![
                say("table", None, "rejected"),
                say("actor", Some("mira"), "one"),
                say("actor", Some("mira"), "two"),
                say("actor", Some("mira"), "three"),
            ],
            intents: vec![
                intent("not_a_move", None),
                intent("wait", None),
                intent("grit", None),
            ],
        };
        let kept = keep_actions(
            actions,
            &SceneScope::Actor {
                actor: "mira".to_string(),
            },
            &menu,
        );
        assert_eq!(
            kept.says.iter().map(SayArgs::text).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(kept.intent.unwrap().move_name, "wait");
    }

    #[test]
    fn actor_wake_drops_a_table_say() {
        let menu = docks().menu;
        let actions = ModelActions {
            says: vec![
                say("table", None, "nope"),
                say("actor", Some("mira"), "yes"),
            ],
            intents: vec![],
        };
        let kept = keep_actions(
            actions,
            &SceneScope::Actor {
                actor: "mira".to_string(),
            },
            &menu,
        );
        assert_eq!(kept.says.len(), 1);
        assert_eq!(kept.says[0].text(), "yes");
    }

    #[test]
    fn queued_wake_keeps_a_table_say_and_drops_an_actor_say() {
        let menu = docks().menu;
        let actions = ModelActions {
            says: vec![
                say("table", None, "here"),
                say("actor", Some("mira"), "aside"),
            ],
            intents: vec![],
        };
        let kept = keep_actions(
            actions,
            &SceneScope::Place {
                place: "docks_shop".to_string(),
            },
            &menu,
        );
        assert_eq!(kept.says.len(), 1);
        assert_eq!(kept.says[0].text(), "here");
    }

    #[test]
    fn sit_with_no_pc_skips_the_model() {
        let pack = docks();
        let mut called = false;
        let prepared = prepare_turn(&pack, &Wake::Queued, &[], &mut |_| {
            called = true;
            Err("called".into())
        });
        match prepared {
            PreparedTurn::NoModel { says } => assert!(says.is_empty()),
            PreparedTurn::Model(_) => panic!("model request"),
        }
        assert!(!called);
    }

    #[test]
    fn different_places_say_they_are_not_here() {
        let pack = docks();
        let wake = Wake::Say {
            from: Speaker::Player("pc_rowan".to_string()),
            seq: 4,
            address: "actor".to_string(),
            place: None,
            actor: Some("mira".to_string()),
            text: "good evening".to_string(),
        };
        let mut calls = Vec::new();
        let prepared = prepare_turn(&pack, &wake, &[], &mut |viewer| {
            calls.push(viewer.to_string());
            Ok(serde_json::json!({
                "viewer": viewer,
                "place": "mira_rooms",
                "visible_actors": ["mira"],
                "known_facts": [],
                "notes": [],
                "unknown_mentions": [],
                "clocks": [],
                "items": [],
                "habits": []
            }))
        });
        match prepared {
            PreparedTurn::NoModel { says } => {
                assert_eq!(says.len(), 1);
                assert_eq!(says[0].text(), "They are not here.");
                assert!(matches!(says[0], SayArgs::Table { .. }));
            }
            PreparedTurn::Model(_) => panic!("model request"),
        }
        assert_eq!(calls, vec!["mira".to_string()]);
    }

    #[test]
    fn the_buffer_keeps_sixteen_and_drops_the_oldest() {
        let mut buffer = Vec::new();
        for index in 0..20 {
            push_handled(
                &mut buffer,
                HandledSay {
                    seq: Some(index),
                    from: Speaker::Player("pc_rowan".to_string()),
                    address: "table".to_string(),
                    place: None,
                    actor: None,
                    text: index.to_string(),
                },
            );
        }
        assert_eq!(buffer.len(), 16);
        assert_eq!(buffer[0].text, "4");
        assert_eq!(buffer[15].text, "19");
    }
}
