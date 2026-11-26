//! Typed YAML for a rule. Serde picks the effect and cond variants.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_saphyr::Spanned;

use crate::ecs::id::{ActorId, PlaceId};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct RuleYaml {
    pub(super) id: Spanned<String>,
    pub(super) when: Spanned<WhenYaml>,
    #[serde(rename = "if")]
    pub(super) if_: Option<IfYaml>,
    pub(super) cost: Option<CostYaml>,
    pub(super) start: Option<ActionStartYaml>,
    pub(super) finish: Option<ActionFinishYaml>,
    /// Ticks the actor stays busy after the finish.
    #[serde(default)]
    pub(super) cooldown: u64,
    /// Whether the actor may cancel the action while it is under way.
    #[serde(default)]
    pub(super) cancellable: bool,
    /// `let:`: each id and its number. A map keeps no order, so `compile::rule` sorts the
    /// numbers by where they are written; an id written twice is the YAML's duplicate key.
    #[serde(rename = "let", default)]
    pub(super) lets: std::collections::BTreeMap<String, Spanned<ExprYaml>>,
    pub(super) check: Option<CheckYaml>,
    #[serde(rename = "do", default)]
    pub(super) do_: Vec<EffectYaml>,
    #[serde(default)]
    pub(super) success: Vec<EffectYaml>,
    #[serde(default)]
    pub(super) partial: Vec<EffectYaml>,
    #[serde(default)]
    pub(super) miss: Vec<EffectYaml>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum WhenYaml {
    Word(String),
    Map(Box<WhenMapYaml>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WhenMapYaml {
    pub verb: Option<String>,
    pub target: Option<String>,
    pub change: Option<ChangeYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChangeYaml {
    pub arrive: Option<String>,
    pub leave: Option<String>,
    pub subject: Option<String>,
    pub meter: Option<String>,
    pub crosses: Option<String>,
    pub flag: Option<String>,
    pub becomes: Option<bool>,
    pub item: Option<String>,
    pub gained: Option<bool>,
    pub lost: Option<bool>,
}

pub(super) enum IfYaml {
    List(Vec<CondYaml>),
    One(Box<CondYaml>),
}

impl<'de> Deserialize<'de> for IfYaml {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IfVisitor;

        impl<'de> Visitor<'de> for IfVisitor {
            type Value = IfYaml;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("one condition or a list of conditions")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<IfYaml, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut list = Vec::new();
                while let Some(cond) = seq.next_element()? {
                    list.push(cond);
                }
                Ok(IfYaml::List(list))
            }

            fn visit_map<A>(self, map: A) -> Result<IfYaml, A::Error>
            where
                A: MapAccess<'de>,
            {
                let cond = CondYaml::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(IfYaml::One(Box::new(cond)))
            }
        }

        deserializer.deserialize_any(IfVisitor)
    }
}

/// One cond; its one key names the cond.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(in crate::canon) enum CondYaml {
    HasItem(HasItemYaml),
    Meter(MeterCondYaml),
    Flag(FlagCondYaml),
    At(AtYaml),
    Cycle(CycleYaml),
    Players(PlayersYaml),
    NoteExists(String),
    Wielding(WieldingYaml),
    All(Vec<CondYaml>),
    Any(Vec<CondYaml>),
    Not(Box<CondYaml>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct HasItemYaml {
    pub(super) subject: Spanned<String>,
    pub(super) id: String,
    /// How many the subject holds at least; 1 when left out.
    pub(super) count: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct WieldingYaml {
    pub(super) subject: Spanned<String>,
    pub(super) id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct MeterCondYaml {
    pub(super) of: Spanned<String>,
    pub(super) id: String,
    pub(super) op: String,
    pub(super) value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct FlagCondYaml {
    pub(super) of: Spanned<String>,
    pub(super) key: String,
    #[serde(rename = "is")]
    pub(super) is_: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct AtYaml {
    pub(super) subject: Spanned<String>,
    pub(super) place: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct CycleYaml {
    pub(super) id: String,
    pub(super) from: Spanned<ExprYaml>,
    pub(super) to: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct PlayersYaml {
    pub(super) op: String,
    pub(super) value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct CostYaml {
    pub(super) ticks: Option<Spanned<ExprYaml>>,
    #[serde(default)]
    pub(super) meters: CountsYaml,
    #[serde(default)]
    pub(super) items: CountsYaml,
}

/// Counts by key under a cost. A plain map, so each count keeps its line.
pub(super) type CountsYaml = std::collections::BTreeMap<String, Spanned<ExprYaml>>;

/// The start of an action. Its cost is the rule's `cost`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionStartYaml {
    #[serde(default)]
    pub counts_as_act: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ActionFinishYaml {
    pub cost: Option<CostYaml>,
    #[serde(default)]
    pub counts_as_act: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckYaml {
    pub total: Spanned<ExprYaml>,
    pub success_at: Spanned<ExprYaml>,
    pub partial_at: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum ExprYaml {
    Literal(i64),
    Text(String),
}

/// One effect under a dash; its one key names the effect.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(super) enum EffectYaml {
    Meter(MeterEffectYaml),
    Flag(FlagEffectYaml),
    Modifier(ModifierEffectYaml),
    Location(LocationYaml),
    Note(NoteYaml),
    Narrate(NarrateYaml),
    Memory(MemoryYaml),
    Item(ItemEffectYaml),
    Wield(WieldYaml),
    Unwield(UnwieldYaml),
    Check(CheckEffectYaml),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeterEffectYaml {
    pub of: Spanned<String>,
    pub key: String,
    pub add: Option<Spanned<ExprYaml>>,
    pub set: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FlagEffectYaml {
    pub of: Spanned<String>,
    pub key: String,
    pub value: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ModifierEffectYaml {
    pub to: Spanned<String>,
    pub id: String,
    pub add: Spanned<ExprYaml>,
    /// How long this application lasts, 1 or more.
    pub ticks: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LocationYaml {
    pub to: Option<String>,
    pub via: Option<String>,
    #[serde(default)]
    pub teleport: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoteYaml {
    pub id: String,
    #[serde(rename = "to", default)]
    pub to_ids: Vec<ActorId>,
    #[serde(rename = "place")]
    pub place_id: Option<PlaceId>,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NarrateYaml {
    pub to: Option<String>,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryYaml {
    pub to: Spanned<String>,
    pub text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ItemEffectYaml {
    pub of: Spanned<String>,
    pub id: String,
    pub add: Spanned<ExprYaml>,
    pub max: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WieldYaml {
    pub subject: Spanned<String>,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UnwieldYaml {
    pub subject: Spanned<String>,
}

/// A nested `check:`: `by` (`self` when left out) rolls `total` against `success_at`, and the
/// outcome picks one of the lists, any of which may be left out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CheckEffectYaml {
    pub by: Option<Spanned<String>>,
    pub total: Spanned<ExprYaml>,
    pub success_at: Spanned<ExprYaml>,
    pub partial_at: Option<Spanned<ExprYaml>>,
    #[serde(default)]
    pub success: Vec<EffectYaml>,
    #[serde(default)]
    pub partial: Vec<EffectYaml>,
    #[serde(default)]
    pub miss: Vec<EffectYaml>,
}
