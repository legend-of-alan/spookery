use std::collections::BTreeMap;

use serde_json::{Map, Value};
use spookery_kernel::canon::{Canon, CastKind};
use spookery_kernel::moves::{Audience, MoveOn, MoveToken};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub name: String,
    pub audience: String,
    pub token: String,
    pub on: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandledSay {
    pub seq: Option<u64>,
    pub from: String,
    pub address: String,
    pub place: Option<String>,
    pub actor: Option<String>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueItem {
    pub id: String,
    pub actor_id: String,
    pub move_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Roll {
    pub faces: Vec<u64>,
    pub stat: i64,
    pub total: i64,
    pub target: i64,
    pub success: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutgoingSay {
    Table { text: String },
    Place { place: String, text: String },
    Actor { actor: String, text: String },
}

impl OutgoingSay {
    pub fn text(&self) -> &str {
        match self {
            OutgoingSay::Table { text }
            | OutgoingSay::Place { text, .. }
            | OutgoingSay::Actor { text, .. } => text,
        }
    }

    pub fn arguments(&self) -> Value {
        match self {
            OutgoingSay::Table { text } => {
                obj([("address", json_str("table")), ("text", json_str(text))])
            }
            OutgoingSay::Place { place, text } => obj([
                ("address", json_str("place")),
                ("place", json_str(place)),
                ("text", json_str(text)),
            ]),
            OutgoingSay::Actor { actor, text } => obj([
                ("address", json_str("actor")),
                ("actor", json_str(actor)),
                ("text", json_str(text)),
            ]),
        }
    }

    pub fn handled(&self, seq: Option<u64>) -> HandledSay {
        match self {
            OutgoingSay::Table { text } => HandledSay {
                seq,
                from: "gm".to_string(),
                address: "table".to_string(),
                place: None,
                actor: None,
                text: text.clone(),
            },
            OutgoingSay::Place { place, text } => HandledSay {
                seq,
                from: "gm".to_string(),
                address: "place".to_string(),
                place: Some(place.clone()),
                actor: None,
                text: text.clone(),
            },
            OutgoingSay::Actor { actor, text } => HandledSay {
                seq,
                from: "gm".to_string(),
                address: "actor".to_string(),
                place: None,
                actor: Some(actor.clone()),
                text: text.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetDraft {
    Empty,
    Place(String),
    Actor(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntentDraft {
    pub actor_id: String,
    pub actor_kind: String,
    pub move_name: String,
    pub target: TargetDraft,
}

impl IntentDraft {
    pub fn arguments(&self) -> Value {
        let target = match &self.target {
            TargetDraft::Empty => Value::Object(Map::new()),
            TargetDraft::Place(place) => obj([("place", json_str(place))]),
            TargetDraft::Actor(actor) => obj([("actor", json_str(actor))]),
        };
        obj([
            ("actor_id", json_str(&self.actor_id)),
            ("actor_kind", json_str(&self.actor_kind)),
            ("source", json_str("player")),
            ("move", json_str(&self.move_name)),
            ("target", target),
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelObject {
    pub says: Vec<OutgoingSay>,
    pub intents: Vec<IntentDraft>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rule {
    Actor { actor: String },
    Place { place: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kept {
    pub says: Vec<OutgoingSay>,
    pub intent: Option<IntentDraft>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Wake {
    Say {
        from: String,
        seq: u64,
        address: String,
        place: Option<String>,
        actor: Option<String>,
        text: String,
        gen: u64,
    },
    Queued {
        gen: u64,
    },
    Sit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnView {
    pub rule: Rule,
    pub player: Option<Value>,
    pub npcs: Vec<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prepared {
    NoModel { says: Vec<OutgoingSay> },
    Model(TurnView),
}

/// Lines and queue that ride with the views. The prompt function does not take a canon.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromptSide {
    pub buffer: Vec<HandledSay>,
    pub new_lines: Vec<HandledSay>,
    pub queue: Vec<QueueItem>,
    pub checks: Vec<Roll>,
}

pub fn from_canon(canon: &Canon) -> (Vec<MenuRow>, BTreeMap<String, CastKind>) {
    let mut menu = Vec::new();
    for (name, def) in &canon.moves {
        let Some(row) = &def.row else {
            continue;
        };
        let Some(audience) = row.audience else {
            continue;
        };
        if audience == Audience::System {
            continue;
        }
        menu.push(MenuRow {
            name: name.clone(),
            audience: audience_name(audience).to_string(),
            token: token_name(row.token).to_string(),
            on: on_name(row.on).to_string(),
        });
    }
    let mut kinds = BTreeMap::new();
    for (id, cast) in &canon.cast {
        kinds.insert(id.clone(), cast.kind);
    }
    (menu, kinds)
}

pub fn push_handled(buffer: &mut Vec<HandledSay>, say: HandledSay) {
    buffer.push(say);
    if buffer.len() > 16 {
        let extra = buffer.len() - 16;
        buffer.drain(0..extra);
    }
}

/// `None` means the JSON is not the action object. That is an empty action list.
pub fn parse_actions(text: &str, says_only: bool) -> Option<ModelObject> {
    let value: Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;
    if says_only {
        if obj.len() != 1 || !obj.contains_key("says") {
            return None;
        }
    } else if obj.len() != 2 || !obj.contains_key("says") || !obj.contains_key("intents") {
        return None;
    }
    let says = parse_says(obj.get("says")?)?;
    let intents = if says_only {
        Vec::new()
    } else {
        parse_intents(obj.get("intents")?)?
    };
    Some(ModelObject { says, intents })
}

pub fn keep(object: ModelObject, rule: &Rule, menu: &[MenuRow]) -> Kept {
    let mut says = Vec::new();
    for say in object.says {
        if !accept_say(&say, rule) {
            continue;
        }
        if says.len() == 2 {
            continue;
        }
        says.push(say);
    }
    let mut intent = None;
    for item in object.intents {
        if !accept_intent(&item, menu) {
            continue;
        }
        if intent.is_some() {
            continue;
        }
        intent = Some(item);
    }
    Kept { says, intent }
}

pub fn prepare(
    canon: &Canon,
    wake: &Wake,
    present: &[String],
    perceive: &mut dyn FnMut(&str) -> Result<Value, String>,
) -> Prepared {
    let (_, kinds) = from_canon(canon);
    prepare_with(&kinds, wake, present, perceive)
}

pub fn prepare_with(
    kinds: &BTreeMap<String, CastKind>,
    wake: &Wake,
    present: &[String],
    perceive: &mut dyn FnMut(&str) -> Result<Value, String>,
) -> Prepared {
    match wake {
        Wake::Say {
            from,
            address,
            actor,
            ..
        } if address == "actor" => {
            let Some(actor) = actor.as_deref().filter(|id| !id.is_empty()) else {
                return absent();
            };
            let speaker = speaker_id(from);
            let as_viewer = match kinds.get(actor) {
                Some(CastKind::Npc) => format!("npc:{actor}"),
                Some(CastKind::PcTemplate) => format!("player:{actor}"),
                None => return absent(),
            };
            let view = match perceive(&as_viewer) {
                Ok(view) => view,
                Err(_) => return absent(),
            };
            if !visible_has(&view, &speaker) {
                return absent();
            }
            match kinds.get(actor) {
                Some(CastKind::Npc) => Prepared::Model(TurnView {
                    rule: Rule::Actor {
                        actor: actor.to_string(),
                    },
                    player: None,
                    npcs: vec![strip_npc(&view)],
                }),
                Some(CastKind::PcTemplate) => Prepared::Model(TurnView {
                    rule: Rule::Actor {
                        actor: actor.to_string(),
                    },
                    player: Some(strip_player(&view)),
                    npcs: Vec::new(),
                }),
                None => absent(),
            }
        }
        Wake::Say {
            from,
            address,
            place,
            ..
        } => {
            let speaker = speaker_id(from);
            let player = match perceive(&format!("player:{speaker}")) {
                Ok(view) => view,
                Err(_) => return Prepared::NoModel { says: Vec::new() },
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
            let npcs = npc_views(&player, kinds, &filter, perceive);
            Prepared::Model(TurnView {
                rule: Rule::Place { place: filter },
                player: Some(strip_player(&player)),
                npcs,
            })
        }
        Wake::Queued { .. } | Wake::Sit => {
            let pcs: Vec<&str> = present
                .iter()
                .filter(|id| kinds.get(id.as_str()) == Some(&CastKind::PcTemplate))
                .map(String::as_str)
                .collect();
            if pcs.len() != 1 {
                return Prepared::NoModel { says: Vec::new() };
            }
            let speaker = pcs[0];
            let player = match perceive(&format!("player:{speaker}")) {
                Ok(view) => view,
                Err(_) => return Prepared::NoModel { says: Vec::new() },
            };
            let place = place_of(&player);
            let npcs = npc_views(&player, kinds, &place, perceive);
            Prepared::Model(TurnView {
                rule: Rule::Place { place },
                player: Some(strip_player(&player)),
                npcs,
            })
        }
    }
}

pub fn render(menu: &[MenuRow], side: &PromptSide, view: &TurnView, checks: &[Roll]) -> String {
    let mut map = Map::new();
    map.insert(
        "menu".to_string(),
        Value::Array(menu.iter().map(menu_value).collect()),
    );
    map.insert(
        "buffer".to_string(),
        Value::Array(side.buffer.iter().map(say_value).collect()),
    );
    map.insert(
        "lines".to_string(),
        Value::Array(side.new_lines.iter().map(say_value).collect()),
    );
    map.insert(
        "queue".to_string(),
        Value::Array(side.queue.iter().map(queue_value).collect()),
    );
    let mut views = Map::new();
    if let Some(player) = &view.player {
        views.insert("player".to_string(), player.clone());
    }
    views.insert("npcs".to_string(), Value::Array(view.npcs.clone()));
    map.insert("views".to_string(), Value::Object(views));
    if !checks.is_empty() {
        map.insert(
            "checks".to_string(),
            Value::Array(checks.iter().map(roll_value).collect()),
        );
    }
    serde_json::to_string(&Value::Object(map)).expect("prompt json")
}

fn absent() -> Prepared {
    Prepared::NoModel {
        says: vec![OutgoingSay::Table {
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
        let Ok(view) = perceive(&format!("npc:{id}")) else {
            continue;
        };
        if view.get("place").and_then(Value::as_str) != Some(place) {
            continue;
        }
        out.push(strip_npc(&view));
    }
    out
}

fn strip_player(view: &Value) -> Value {
    obj([
        ("viewer", view.get("viewer").cloned().unwrap_or(Value::Null)),
        ("place", view.get("place").cloned().unwrap_or(Value::Null)),
        (
            "visible_actors",
            view.get("visible_actors")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        (
            "clocks",
            view.get("clocks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        (
            "stocks",
            view.get("stocks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        (
            "habits",
            view.get("habits")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
    ])
}

fn strip_npc(view: &Value) -> Value {
    let facts = view
        .get("known_facts")
        .and_then(Value::as_array)
        .map(|facts| {
            facts
                .iter()
                .map(|fact| {
                    obj([
                        ("id", fact.get("id").cloned().unwrap_or(Value::Null)),
                        ("text", fact.get("text").cloned().unwrap_or(Value::Null)),
                    ])
                })
                .collect()
        })
        .unwrap_or_default();
    obj([
        ("viewer", view.get("viewer").cloned().unwrap_or(Value::Null)),
        ("place", view.get("place").cloned().unwrap_or(Value::Null)),
        (
            "visible_actors",
            view.get("visible_actors")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        ("facts", Value::Array(facts)),
        (
            "messages",
            view.get("messages")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        (
            "clocks",
            view.get("clocks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        (
            "stocks",
            view.get("stocks")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
        (
            "habits",
            view.get("habits")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
        ),
    ])
}

fn accept_say(say: &OutgoingSay, rule: &Rule) -> bool {
    match rule {
        Rule::Actor { actor } => matches!(say, OutgoingSay::Actor { actor: id, .. } if id == actor),
        Rule::Place { place } => match say {
            OutgoingSay::Table { .. } => true,
            OutgoingSay::Place { place: named, .. } => named == place,
            OutgoingSay::Actor { .. } => false,
        },
    }
}

fn accept_intent(intent: &IntentDraft, menu: &[MenuRow]) -> bool {
    let Some(row) = menu.iter().find(|row| row.name == intent.move_name) else {
        return false;
    };
    matches!(
        (row.token.as_str(), &intent.target),
        ("none", TargetDraft::Empty)
            | ("place", TargetDraft::Place(_))
            | ("actor", TargetDraft::Actor(_))
    )
}

fn parse_says(value: &Value) -> Option<Vec<OutgoingSay>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        out.push(parse_say(item)?);
    }
    Some(out)
}

fn parse_say(value: &Value) -> Option<OutgoingSay> {
    let obj = value.as_object()?;
    let address = obj.get("address")?.as_str()?;
    let text = obj.get("text")?.as_str()?.to_string();
    match address {
        "table" => {
            if obj.len() != 2 {
                return None;
            }
            Some(OutgoingSay::Table { text })
        }
        "place" => {
            if obj.len() != 3 {
                return None;
            }
            let place = obj.get("place")?.as_str()?.to_string();
            if place.is_empty() {
                return None;
            }
            Some(OutgoingSay::Place { place, text })
        }
        "actor" => {
            if obj.len() != 3 {
                return None;
            }
            let actor = obj.get("actor")?.as_str()?.to_string();
            if actor.is_empty() {
                return None;
            }
            Some(OutgoingSay::Actor { actor, text })
        }
        _ => None,
    }
}

fn parse_intents(value: &Value) -> Option<Vec<IntentDraft>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        out.push(parse_intent(item)?);
    }
    Some(out)
}

fn parse_intent(value: &Value) -> Option<IntentDraft> {
    let obj = value.as_object()?;
    if obj.len() != 4 {
        return None;
    }
    let actor_id = obj.get("actor_id")?.as_str()?.to_string();
    let actor_kind = obj.get("actor_kind")?.as_str()?;
    if actor_kind != "npc" && actor_kind != "player" {
        return None;
    }
    let move_name = obj.get("move")?.as_str()?.to_string();
    let target = parse_target(obj.get("target")?)?;
    if actor_id.is_empty() || move_name.is_empty() {
        return None;
    }
    Some(IntentDraft {
        actor_id,
        actor_kind: actor_kind.to_string(),
        move_name,
        target,
    })
}

fn parse_target(value: &Value) -> Option<TargetDraft> {
    let obj = value.as_object()?;
    if obj.is_empty() {
        return Some(TargetDraft::Empty);
    }
    if obj.len() != 1 {
        return None;
    }
    if let Some(place) = obj.get("place").and_then(Value::as_str) {
        if place.is_empty() {
            return None;
        }
        return Some(TargetDraft::Place(place.to_string()));
    }
    if let Some(actor) = obj.get("actor").and_then(Value::as_str) {
        if actor.is_empty() {
            return None;
        }
        return Some(TargetDraft::Actor(actor.to_string()));
    }
    None
}

fn visible_has(view: &Value, id: &str) -> bool {
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

fn speaker_id(from: &str) -> String {
    from.strip_prefix("player:")
        .filter(|id| !id.is_empty())
        .unwrap_or("")
        .to_string()
}

fn menu_value(row: &MenuRow) -> Value {
    obj([
        ("name", json_str(&row.name)),
        ("audience", json_str(&row.audience)),
        ("token", json_str(&row.token)),
        ("on", json_str(&row.on)),
    ])
}

fn say_value(say: &HandledSay) -> Value {
    let mut map = Map::new();
    if let Some(seq) = say.seq {
        map.insert("seq".to_string(), Value::from(seq));
    }
    map.insert("from".to_string(), json_str(&say.from));
    map.insert("address".to_string(), json_str(&say.address));
    if let Some(place) = &say.place {
        map.insert("place".to_string(), json_str(place));
    }
    if let Some(actor) = &say.actor {
        map.insert("actor".to_string(), json_str(actor));
    }
    map.insert("text".to_string(), json_str(&say.text));
    Value::Object(map)
}

fn queue_value(item: &QueueItem) -> Value {
    obj([
        ("id", json_str(&item.id)),
        ("actor_id", json_str(&item.actor_id)),
        ("move", json_str(&item.move_name)),
    ])
}

fn roll_value(roll: &Roll) -> Value {
    obj([
        (
            "faces",
            Value::Array(roll.faces.iter().copied().map(Value::from).collect()),
        ),
        ("stat", Value::from(roll.stat)),
        ("total", Value::from(roll.total)),
        ("target", Value::from(roll.target)),
        ("success", Value::Bool(roll.success)),
    ])
}

fn audience_name(audience: Audience) -> &'static str {
    match audience {
        Audience::Player => "player",
        Audience::Habit => "habit",
        Audience::System => "system",
    }
}

fn token_name(token: MoveToken) -> &'static str {
    match token {
        MoveToken::None => "none",
        MoveToken::Actor => "actor",
        MoveToken::Place => "place",
    }
}

fn on_name(on: MoveOn) -> &'static str {
    match on {
        MoveOn::Speaker => "speaker",
        MoveOn::Named => "named",
    }
}

fn obj<const N: usize>(entries: [(&str, Value); N]) -> Value {
    let mut map = Map::new();
    for (key, value) in entries {
        map.insert(key.to_string(), value);
    }
    Value::Object(map)
}

fn json_str(text: &str) -> Value {
    Value::String(text.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use spookery_kernel::canon::{load_canon, CastKind};
    use spookery_kernel::moves::{ActionRow, Audience, MoveDef, MoveOn, MoveToken};

    use super::*;

    fn docks() -> Canon {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packs/oneshot-docks");
        load_canon(&path).expect("docks canon")
    }

    fn say(address: &str, actor: Option<&str>, text: &str) -> OutgoingSay {
        match address {
            "table" => OutgoingSay::Table {
                text: text.to_string(),
            },
            "actor" => OutgoingSay::Actor {
                actor: actor.unwrap().to_string(),
                text: text.to_string(),
            },
            "place" => OutgoingSay::Place {
                place: actor.unwrap().to_string(),
                text: text.to_string(),
            },
            _ => panic!("address"),
        }
    }

    fn intent(move_name: &str, target: TargetDraft) -> IntentDraft {
        IntentDraft {
            actor_id: "mira".to_string(),
            actor_kind: "npc".to_string(),
            move_name: move_name.to_string(),
            target,
        }
    }

    #[test]
    fn prompt_omits_dropped_strings_and_keeps_fact_text() {
        let mut canon = docks();
        canon.cast.get_mut("mira").unwrap().body = "CAST-BODY-SENTINEL".to_string();
        let (menu, _) = from_canon(&canon);
        let wake = Wake::Say {
            from: "player:pc_rowan".to_string(),
            seq: 1,
            address: "table".to_string(),
            place: None,
            actor: None,
            text: "hello".to_string(),
            gen: 0,
        };
        let mut calls = 0;
        let prepared = prepare(&canon, &wake, &[], &mut |viewer| {
            calls += 1;
            if viewer == "player:pc_rowan" {
                return Ok(serde_json::json!({
                    "viewer": "player:pc_rowan",
                    "place": "docks_shop",
                    "visible_actors": ["aldric", "pc_rowan"],
                    "known_facts": [{"id": "k", "text": "KNOWN-SENTINEL", "body": "KNOWN-BODY"}],
                    "messages": [{"id": "m", "text": "MESSAGE-SENTINEL"}],
                    "unknown_mentioned": ["UNKNOWN-SENTINEL"],
                    "clocks": [],
                    "stocks": [],
                    "habits": []
                }));
            }
            if viewer == "npc:aldric" {
                return Ok(serde_json::json!({
                    "viewer": "npc:aldric",
                    "place": "docks_shop",
                    "visible_actors": ["aldric", "pc_rowan"],
                    "known_facts": [{"id": "f", "text": "FACT-TEXT-SENTINEL", "body": "BODY-SENTINEL"}],
                    "messages": [],
                    "unknown_mentioned": [],
                    "clocks": [],
                    "stocks": [],
                    "habits": []
                }));
            }
            Err("no".into())
        });
        let Prepared::Model(view) = prepared else {
            panic!("model was not called");
        };
        let prompt = render(&menu, &PromptSide::default(), &view, &[]);
        assert!(!prompt.contains("KNOWN-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("BODY-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("CAST-BODY-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("MESSAGE-SENTINEL"), "{prompt}");
        assert!(!prompt.contains("UNKNOWN-SENTINEL"), "{prompt}");
        assert!(prompt.contains("FACT-TEXT-SENTINEL"), "{prompt}");
        assert!(calls >= 2, "player and npc views");
    }

    #[test]
    fn address_rule_runs_before_the_cap() {
        let (menu, _) = from_canon(&docks());
        let object = ModelObject {
            says: vec![
                say("table", None, "rejected"),
                say("actor", Some("mira"), "one"),
                say("actor", Some("mira"), "two"),
                say("actor", Some("mira"), "three"),
            ],
            intents: vec![
                intent("not_a_move", TargetDraft::Empty),
                intent("wait", TargetDraft::Empty),
                intent("grit", TargetDraft::Empty),
            ],
        };
        let kept = keep(
            object,
            &Rule::Actor {
                actor: "mira".to_string(),
            },
            &menu,
        );
        assert_eq!(
            kept.says.iter().map(OutgoingSay::text).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(kept.intent.unwrap().move_name, "wait");
    }

    #[test]
    fn actor_wake_drops_a_table_say() {
        let (menu, _) = from_canon(&docks());
        let object = ModelObject {
            says: vec![
                say("table", None, "nope"),
                say("actor", Some("mira"), "yes"),
            ],
            intents: vec![],
        };
        let kept = keep(
            object,
            &Rule::Actor {
                actor: "mira".to_string(),
            },
            &menu,
        );
        assert_eq!(kept.says.len(), 1);
        assert_eq!(kept.says[0].text(), "yes");
    }

    #[test]
    fn queued_wake_keeps_a_table_say_and_drops_an_actor_say() {
        let (menu, _) = from_canon(&docks());
        let object = ModelObject {
            says: vec![
                say("table", None, "here"),
                say("actor", Some("mira"), "aside"),
            ],
            intents: vec![],
        };
        let kept = keep(
            object,
            &Rule::Place {
                place: "docks_shop".to_string(),
            },
            &menu,
        );
        assert_eq!(kept.says.len(), 1);
        assert_eq!(kept.says[0].text(), "here");
    }

    #[test]
    fn sit_with_no_pc_skips_the_model() {
        let canon = docks();
        let mut called = false;
        let prepared = prepare(&canon, &Wake::Sit, &[], &mut |_| {
            called = true;
            Err("called".into())
        });
        match prepared {
            Prepared::NoModel { says } => assert!(says.is_empty()),
            Prepared::Model(_) => panic!("model request"),
        }
        assert!(!called);
    }

    #[test]
    fn different_places_say_they_are_not_here() {
        let canon = docks();
        let wake = Wake::Say {
            from: "player:pc_rowan".to_string(),
            seq: 4,
            address: "actor".to_string(),
            place: None,
            actor: Some("mira".to_string()),
            text: "good evening".to_string(),
            gen: 0,
        };
        let mut calls = Vec::new();
        let prepared = prepare(&canon, &wake, &[], &mut |viewer| {
            calls.push(viewer.to_string());
            Ok(serde_json::json!({
                "viewer": viewer,
                "place": "mira_rooms",
                "visible_actors": ["mira"],
                "known_facts": [],
                "messages": [],
                "unknown_mentioned": [],
                "clocks": [],
                "stocks": [],
                "habits": []
            }))
        });
        match prepared {
            Prepared::NoModel { says } => {
                assert_eq!(says.len(), 1);
                assert_eq!(says[0].text(), "They are not here.");
                assert!(matches!(says[0], OutgoingSay::Table { .. }));
            }
            Prepared::Model(_) => panic!("model request"),
        }
        assert_eq!(calls, vec!["npc:mira".to_string()]);
    }

    #[test]
    fn system_rows_are_omitted_and_kinds_are_kept() {
        let mut canon = docks();
        canon.moves.insert(
            "hidden_system".to_string(),
            MoveDef {
                check: None,
                row: Some(ActionRow {
                    token: MoveToken::None,
                    on: MoveOn::Speaker,
                    effects: Vec::new(),
                    success: Vec::new(),
                    failure: Vec::new(),
                    item: None,
                    audience: Some(Audience::System),
                    difficulty: 0,
                }),
            },
        );
        canon.cast.get_mut("mira").unwrap().body = "CAST-BODY-SENTINEL".to_string();
        let (menu, kinds) = from_canon(&canon);
        assert!(menu.iter().all(|row| row.audience != "system"));
        assert!(menu.iter().any(|row| row.name == "travel"
            && row.audience == "player"
            && row.token == "place"
            && row.on == "speaker"));
        assert!(menu
            .iter()
            .any(|row| row.name == "restock_lamp_oil" && row.audience == "habit"));
        assert_eq!(kinds.get("mira"), Some(&CastKind::Npc));
        assert_eq!(kinds.get("pc_rowan"), Some(&CastKind::PcTemplate));
        let rendered = format!("{menu:?}{kinds:?}");
        assert!(!rendered.contains("CAST-BODY-SENTINEL"));
    }

    #[test]
    fn the_buffer_keeps_sixteen_and_drops_the_oldest() {
        let mut buffer = Vec::new();
        for index in 0..20 {
            push_handled(
                &mut buffer,
                HandledSay {
                    seq: Some(index),
                    from: "player:pc_rowan".to_string(),
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
