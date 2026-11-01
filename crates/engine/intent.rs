//! Who is asking, and the action they asked the round to take.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::rule::TargetKind;
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Viewer {
    Player(String),
    Npc(String),
}

impl Viewer {
    pub fn parse(raw: &str) -> Result<Self, Reason> {
        let Some((kind, id)) = raw.split_once(':') else {
            return Err(Reason::UnscopedPerceive);
        };
        if id.is_empty() {
            return Err(Reason::UnscopedPerceive);
        }
        match kind {
            "player" => Ok(Viewer::Player(id.to_string())),
            "npc" => Ok(Viewer::Npc(id.to_string())),
            _ => Err(Reason::UnscopedPerceive),
        }
    }

    pub fn actor_id(&self) -> &str {
        match self {
            Viewer::Player(id) | Viewer::Npc(id) => id,
        }
    }
}

impl fmt::Display for Viewer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Viewer::Player(id) => write!(f, "player:{id}"),
            Viewer::Npc(id) => write!(f, "npc:{id}"),
        }
    }
}

impl From<Viewer> for String {
    fn from(id: Viewer) -> Self {
        id.to_string()
    }
}

impl TryFrom<String> for Viewer {
    type Error = Reason;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        Viewer::parse(&raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Engine,
    Player,
    System,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Engine => "engine",
            Source::Player => "player",
            Source::System => "system",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "TargetWire", into = "TargetWire")]
pub enum Target {
    #[default]
    Empty,
    Place(String),
    Exit(String),
    Actor(String),
    Item(String),
    Clock(String),
}

impl Target {
    pub fn new(kind: TargetKind, token: &str) -> Self {
        let token = token.to_string();
        match kind {
            TargetKind::None => Target::Empty,
            TargetKind::Place => Target::Place(token),
            TargetKind::Exit => Target::Exit(token),
            TargetKind::Actor => Target::Actor(token),
            TargetKind::Item => Target::Item(token),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Target::Empty)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IntentArgs {
    pub message_to: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub id: String,
    pub round_index: u64,
    pub actor: Viewer,
    pub source: Source,
    #[serde(rename = "move")]
    pub move_name: String,
    pub target: Target,
    #[serde(with = "args_wire")]
    pub args: IntentArgs,
    pub rationale_code: String,
}

impl Intent {
    /// An intent with no message recipients and no rationale.
    pub(crate) fn new(
        id: String,
        round_index: u64,
        actor: Viewer,
        source: Source,
        move_name: &str,
        target: Target,
    ) -> Self {
        Self {
            id,
            round_index,
            actor,
            source,
            move_name: move_name.to_string(),
            target,
            args: IntentArgs::default(),
            rationale_code: String::new(),
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TargetWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    place: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    item: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clock: Option<String>,
}

impl From<Target> for TargetWire {
    fn from(target: Target) -> Self {
        let mut wire = Self::default();
        match target {
            Target::Empty => {}
            Target::Place(id) => wire.place = Some(id),
            Target::Exit(id) => wire.exit = Some(id),
            Target::Actor(id) => wire.actor = Some(id),
            Target::Item(id) => wire.item = Some(id),
            Target::Clock(id) => wire.clock = Some(id),
        }
        wire
    }
}

impl TryFrom<TargetWire> for Target {
    type Error = &'static str;

    /// At most one key is set.
    fn try_from(wire: TargetWire) -> Result<Self, Self::Error> {
        let mut set = [
            wire.place.map(Target::Place),
            wire.exit.map(Target::Exit),
            wire.actor.map(Target::Actor),
            wire.item.map(Target::Item),
            wire.clock.map(Target::Clock),
        ]
        .into_iter()
        .flatten();
        match (set.next(), set.next()) {
            (None, _) => Ok(Target::Empty),
            (Some(target), None) => Ok(target),
            (Some(_), Some(_)) => Err("target"),
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ArgsWire {
    #[serde(rename = "to", default, skip_serializing_if = "Option::is_none")]
    message_to: Option<Vec<String>>,
}

impl From<&IntentArgs> for ArgsWire {
    fn from(args: &IntentArgs) -> Self {
        Self {
            message_to: args.message_to.clone(),
        }
    }
}

impl From<ArgsWire> for IntentArgs {
    fn from(wire: ArgsWire) -> Self {
        Self {
            message_to: wire.message_to,
        }
    }
}

pub(crate) mod args_wire {
    use super::ArgsWire;
    use crate::intent::IntentArgs;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(crate) fn serialize<S: Serializer>(
        args: &IntentArgs,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        ArgsWire::from(args).serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<IntentArgs, D::Error> {
        Ok(IntentArgs::from(ArgsWire::deserialize(deserializer)?))
    }
}

/// Blake2s of `world|round|actor|row`. The P37 preimage stays.
pub(crate) fn engine_intent_id(
    world_id: &str,
    round_index: u64,
    actor_id: &str,
    row: &str,
) -> String {
    crate::hash::blake2s_hex(format!("{world_id}|{round_index}|{actor_id}|{row}").as_bytes())
}

/// Blake2s of `world|round_index_plus_one|actor|join_move`.
pub(crate) fn player_intent_id(
    world_id: &str,
    round_index_plus_one: u64,
    actor_id: &str,
    line: &str,
) -> String {
    crate::hash::blake2s_hex(
        format!("{world_id}|{round_index_plus_one}|{actor_id}|{line}").as_bytes(),
    )
}

/// The id of the intent that finishes `actor_id`'s timed action.
pub(crate) fn due_intent_id(actor_id: &str) -> String {
    format!("arrival:{actor_id}")
}

/// The id of the intent that fires `rule_id` for `actor_id`.
pub(crate) fn trigger_intent_id(rule_id: &str, actor_id: &str) -> String {
    format!("trigger:{rule_id}:{actor_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_intent_id_is_pinned() {
        assert_eq!(
            engine_intent_id("cap", 1, "n2", "work"),
            "daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819"
        );
    }
}
