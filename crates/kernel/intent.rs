use serde::{Deserialize, Serialize};

use crate::error::Reason;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum ViewerId {
    Player(String),
    Npc(String),
}

impl ViewerId {
    pub fn parse(raw: &str) -> Result<Self, Reason> {
        let Some((kind, id)) = raw.split_once(':') else {
            return Err(Reason::UnscopedPerceive);
        };
        if id.is_empty() {
            return Err(Reason::UnscopedPerceive);
        }
        match kind {
            "player" => Ok(ViewerId::Player(id.to_string())),
            "npc" => Ok(ViewerId::Npc(id.to_string())),
            _ => Err(Reason::UnscopedPerceive),
        }
    }

    pub fn actor_id(&self) -> &str {
        match self {
            ViewerId::Player(id) | ViewerId::Npc(id) => id,
        }
    }

    pub fn token(&self) -> String {
        match self {
            ViewerId::Player(id) => format!("player:{id}"),
            ViewerId::Npc(id) => format!("npc:{id}"),
        }
    }

    pub fn from_kind(kind: ActorKind, id: &str) -> Result<Self, Reason> {
        match kind {
            ActorKind::Npc => Ok(ViewerId::Npc(id.to_string())),
            ActorKind::Player => Ok(ViewerId::Player(id.to_string())),
            ActorKind::System => Err(Reason::UnscopedPerceive),
        }
    }
}

impl From<ViewerId> for String {
    fn from(id: ViewerId) -> Self {
        id.token()
    }
}

impl TryFrom<String> for ViewerId {
    type Error = Reason;

    fn try_from(raw: String) -> Result<Self, Self::Error> {
        ViewerId::parse(&raw)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Npc,
    Player,
    System,
}

impl ActorKind {
    pub fn token(self) -> &'static str {
        match self {
            ActorKind::Npc => "npc",
            ActorKind::Player => "player",
            ActorKind::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Kernel,
    Player,
    System,
}

impl Source {
    pub fn token(self) -> &'static str {
        match self {
            Source::Kernel => "kernel",
            Source::Player => "player",
            Source::System => "system",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Target {
    #[default]
    Empty,
    Place(String),
    Actor(String),
    Clock(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct IntentArgs {
    pub message_to: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub id: String,
    pub tick_index: u64,
    pub actor_id: String,
    pub actor_kind: ActorKind,
    pub source: Source,
    #[serde(rename = "move")]
    pub move_name: String,
    #[serde(with = "crate::store::records::target_as")]
    pub target: Target,
    #[serde(with = "crate::store::records::args_as")]
    pub args: IntentArgs,
    pub rationale_code: String,
}
