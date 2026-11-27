//! Who is asking, and the action they asked the round to take.

use serde::{Deserialize, Serialize};

use crate::ecs;
use crate::ecs::id::{ActorId, PathId, PlaceId};
use crate::rules::rule::TargetKind;
use crate::Reason;

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
    Path(PathId),
    Actor(ActorId),
    Item(String),
}

impl Target {
    pub fn new(kind: TargetKind, token: &str) -> Self {
        let token = token.to_string();
        match kind {
            TargetKind::None => Target::Empty,
            TargetKind::Place => Target::Place(token.into()),
            TargetKind::Path => Target::Path(token.into()),
            TargetKind::Actor => Target::Actor(token.into()),
            TargetKind::Item => Target::Item(token),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Target::Empty)
    }

    /// The id a verb names.
    pub(crate) fn token(&self) -> Option<&str> {
        match self {
            Target::Place(id) => Some(id.as_str()),
            Target::Actor(id) => Some(id.as_str()),
            Target::Path(id) => Some(id.as_str()),
            Target::Item(id) => Some(id),
            Target::Empty => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    // repeats module: re-exported at the crate root
    #[serde(rename = "to", default, skip_serializing_if = "Option::is_none")]
    pub message_to: Option<Vec<ActorId>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub id: String,
    pub round_index: u64,
    /// Who acts. A change rule that watches a world change runs with no one.
    #[serde(rename = "actor", default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<ActorId>,
    pub source: Source,
    #[serde(rename = "verb")]
    pub verb_id: String,
    pub target: Target,
    pub args: Args,
    pub rationale_code: String,
}

impl Intent {
    /// An intent with no message recipients and no rationale.
    pub(crate) fn new(
        id: String,
        round_index: u64,
        actor_id: Option<ActorId>,
        source: Source,
        verb_id: &str,
        target: Target,
    ) -> Self {
        Self {
            id,
            round_index,
            actor_id,
            source,
            verb_id: verb_id.to_string(),
            target,
            args: Args::default(),
            rationale_code: String::new(),
        }
    }

    /// `actor_id`'s line `verb token` for the next round. The id is the line's, so the queue
    /// can refuse it by id before the rule it picks shapes the target, which starts empty.
    pub(crate) fn player(
        world: &ecs::World,
        actor_id: &ActorId,
        verb: &str,
        token: Option<&str>,
    ) -> Result<Self, Reason> {
        let round = world.round_index.checked_add(1).ok_or(Reason::Schema)?;
        let line = match token {
            Some(token) => format!("{verb} {token}"),
            None => verb.to_string(),
        };
        Ok(Self::new(
            id(&world.world_id, round, actor_id.as_str(), &line),
            round,
            Some(actor_id.clone()),
            Source::Player,
            verb,
            Target::Empty,
        ))
    }

    /// The engine running `actor_id`'s habit `local_id` this round.
    pub(crate) fn habit(world: &ecs::World, actor_id: &ActorId, local_id: &str) -> Self {
        let round = world.round_index;
        Self::new(
            id(&world.world_id, round, actor_id.as_str(), local_id),
            round,
            Some(actor_id.clone()),
            Source::Engine,
            local_id,
            Target::Empty,
        )
    }

    /// The engine firing change rule `rule_id` this round: for `subject` as
    /// `trigger:<rule>:<actor>`, or for a world change, with no subject, as `trigger:<rule>`.
    pub(crate) fn trigger(
        world: &ecs::World,
        subject: Option<&ActorId>,
        rule_id: &str,
    ) -> Result<Self, Reason> {
        let (actor_id, id) = match subject {
            Some(actor_id) => {
                if !world.actors().contains(actor_id) {
                    return Err(Reason::UnknownId);
                }
                (
                    Some(actor_id.clone()),
                    format!("trigger:{rule_id}:{actor_id}"),
                )
            }
            None => (None, format!("trigger:{rule_id}")),
        };
        let intent = Self::new(
            id,
            world.round_index,
            actor_id,
            Source::Engine,
            rule_id,
            Target::Empty,
        );
        Ok(intent)
    }

    /// The finish of `actor_id`'s timed action, as `finish:<actor>`, with the source, verb
    /// and target it began with.
    pub(crate) fn finish(
        world: &ecs::World,
        actor_id: &ActorId,
        source: Source,
        verb: &str,
        target: Target,
    ) -> Result<Self, Reason> {
        if !world.actors().contains(actor_id) {
            return Err(Reason::UnknownId);
        }
        let id = format!("finish:{actor_id}");
        Ok(Self::new(
            id,
            world.round_index,
            Some(actor_id.clone()),
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
    path: Option<PathId>,
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
            Target::Path(id) => wire.path = Some(id),
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
            wire.path.map(Target::Path),
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
fn id(world_id: &str, round_index: u64, actor_id: &str, line: &str) -> String {
    crate::hash::blake2s_hex(format!("{world_id}|{round_index}|{actor_id}|{line}").as_bytes())
}
