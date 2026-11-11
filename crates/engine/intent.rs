//! Who is asking, and the action they asked the round to take.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize};

use crate::rules::rule::TargetKind;
use crate::state::id::{ActorId, PlaceId};
use crate::state::State;
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub enum Viewer {
    Player(ActorId),
    Npc(ActorId),
}

/// `player:<id>` or `npc:<id>`.
impl FromStr for Viewer {
    type Err = Reason;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let Some((kind, id)) = raw.split_once(':') else {
            return Err(Reason::UnscopedPerceive);
        };
        if id.is_empty() {
            return Err(Reason::UnscopedPerceive);
        }
        match kind {
            "player" => Ok(Viewer::Player(id.into())),
            "npc" => Ok(Viewer::Npc(id.into())),
            _ => Err(Reason::UnscopedPerceive),
        }
    }
}

impl Viewer {
    pub fn actor_id(&self) -> &ActorId {
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

impl<'de> Deserialize<'de> for Viewer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Engine,
    Player,
    Gm,
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(try_from = "TargetWire", into = "TargetWire")]
pub enum Target {
    #[default]
    Empty,
    Place(PlaceId),
    Exit(String),
    Actor(ActorId),
    Item(String),
}

impl Target {
    pub fn new(kind: TargetKind, token: &str) -> Self {
        let token = token.to_string();
        match kind {
            TargetKind::None => Target::Empty,
            TargetKind::Place => Target::Place(token.into()),
            TargetKind::Exit => Target::Exit(token),
            TargetKind::Actor => Target::Actor(token.into()),
            TargetKind::Item => Target::Item(token),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Target::Empty)
    }

    /// The id a move names.
    pub(crate) fn token(&self) -> Option<&str> {
        match self {
            Target::Place(id) => Some(id.as_str()),
            Target::Actor(id) => Some(id.as_str()),
            Target::Exit(id) | Target::Item(id) => Some(id),
            Target::Empty => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntentArgs {
    #[serde(rename = "to", default, skip_serializing_if = "Option::is_none")]
    pub message_to: Option<Vec<ActorId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub id: String,
    pub round_index: u64,
    /// Who acts. A change rule that watches a world change runs with no one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Viewer>,
    pub source: Source,
    #[serde(rename = "move")]
    pub move_name: String,
    pub target: Target,
    pub args: IntentArgs,
    pub rationale_code: String,
}

impl Intent {
    /// An intent with no message recipients and no rationale.
    pub(crate) fn new(
        id: String,
        round_index: u64,
        actor: Option<Viewer>,
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

    /// The acting actor's id, or `None` for a change rule on a world change.
    pub fn actor_id(&self) -> Option<&ActorId> {
        self.actor.as_ref().map(Viewer::actor_id)
    }

    /// `actor_id`'s line `verb token` for the next round. The id is the line's, so the queue
    /// can refuse it by id before the rule it picks shapes the target, which starts empty.
    pub(crate) fn player(
        state: &State,
        actor_id: &ActorId,
        verb: &str,
        token: Option<&str>,
    ) -> Result<Self, Reason> {
        let round = state.round_index.checked_add(1).ok_or(Reason::Schema)?;
        let line = match token {
            Some(token) => format!("{verb} {token}"),
            None => verb.to_string(),
        };
        let id = intent_id(&state.world_id, round, actor_id.as_str(), &line);
        let actor = Some(Viewer::Player(actor_id.clone()));
        let intent = Self::new(id, round, actor, Source::Player, verb, Target::Empty);
        Ok(intent)
    }

    /// The engine running `actor_id`'s habit `local_id` this round.
    pub(crate) fn habit(state: &State, actor_id: &ActorId, local_id: &str) -> Self {
        let (round, npc) = (state.round_index, Some(Viewer::Npc(actor_id.clone())));
        let id = intent_id(&state.world_id, round, actor_id.as_str(), local_id);
        Self::new(id, round, npc, Source::Engine, local_id, Target::Empty)
    }

    /// The engine firing change rule `rule_id` this round: for `subject` as
    /// `trigger:<rule>:<actor>`, or for a world change, with no subject, as `trigger:<rule>`.
    pub(crate) fn trigger(
        state: &State,
        subject: Option<&ActorId>,
        rule_id: &str,
    ) -> Result<Self, Reason> {
        let (actor, id) = match subject {
            Some(actor_id) => (
                Some(state.viewer_of(actor_id).ok_or(Reason::UnknownId)?),
                format!("trigger:{rule_id}:{actor_id}"),
            ),
            None => (None, format!("trigger:{rule_id}")),
        };
        let intent = Self::new(
            id,
            state.round_index,
            actor,
            Source::Engine,
            rule_id,
            Target::Empty,
        );
        Ok(intent)
    }

    /// The finish of `actor_id`'s timed action, as `finish:<actor>`, with the source, verb
    /// and target it began with.
    pub(crate) fn finish(
        state: &State,
        actor_id: &ActorId,
        source: Source,
        verb: &str,
        target: Target,
    ) -> Result<Self, Reason> {
        let actor = Some(state.viewer_of(actor_id).ok_or(Reason::UnknownId)?);
        let id = format!("finish:{actor_id}");
        Ok(Self::new(
            id,
            state.round_index,
            actor,
            source,
            verb,
            target,
        ))
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetWire {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    place: Option<PlaceId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<ActorId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    item: Option<String>,
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

/// Blake2s of `world|round|actor|line`: a player's line for the round after the save's,
/// or a habit's local id for the save's round.
pub(crate) fn intent_id(world_id: &str, round_index: u64, actor_id: &str, line: &str) -> String {
    crate::hash::blake2s_hex(format!("{world_id}|{round_index}|{actor_id}|{line}").as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_id_is_pinned() {
        assert_eq!(
            intent_id("cap", 1, "n2", "work"),
            "daa186f67613ae8ea1275b3fd03c38d52eea99e976235e499b5c2b0853c0d819"
        );
    }
}
