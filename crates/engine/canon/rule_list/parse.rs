//! Typed YAML for a rule. Serde picks the effect and condition variants.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_saphyr::Spanned;

use crate::ecs::id::{ActorId, PlaceId};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct RuleYaml {
    pub id: Spanned<String>,
    pub when: Spanned<WhenYaml>,
    #[serde(rename = "if")]
    pub if_: Option<IfYaml>,
    pub cost: Option<CostYaml>,
    pub start: Option<StartYaml>,
    pub finish: Option<FinishYaml>,
    /// Ticks the actor stays busy after the finish.
    #[serde(default)]
    pub cooldown: u64,
    /// Whether the actor may cancel the action while it is under way.
    #[serde(default)]
    pub cancellable: bool,
    /// `let:`: each id and its number. A map keeps no order, so `rule_of` sorts the
    /// numbers by where they are written; an id written twice is the YAML's duplicate key.
    #[serde(rename = "let", default)]
    pub lets: std::collections::BTreeMap<String, Spanned<ExprYaml>>,
    pub check: Option<CheckYaml>,
    #[serde(rename = "do", default)]
    pub do_: Vec<EffectYaml>,
    #[serde(default)]
    pub success: Vec<EffectYaml>,
    #[serde(default)]
    pub partial: Vec<EffectYaml>,
    #[serde(default)]
    pub miss: Vec<EffectYaml>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(in crate::canon) enum WhenYaml {
    Word(String),
    Map(Box<WhenMap>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct WhenMap {
    pub verb: Option<String>,
    pub target: Option<String>,
    pub change: Option<ChangeYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct ChangeYaml {
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

pub(in crate::canon) enum IfYaml {
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

/// One condition; its one key names the condition.
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
    pub subject: Spanned<String>,
    pub id: String,
    /// How many the subject holds at least; 1 when left out.
    pub count: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct WieldingYaml {
    pub subject: Spanned<String>,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct MeterCondYaml {
    pub of: Spanned<String>,
    pub id: String,
    pub op: String,
    pub value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct FlagCondYaml {
    pub of: Spanned<String>,
    pub key: String,
    #[serde(rename = "is")]
    pub is_: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct AtYaml {
    pub subject: Spanned<String>,
    pub place: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct CycleYaml {
    pub id: String,
    pub from: Spanned<ExprYaml>,
    pub to: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct PlayersYaml {
    pub op: String,
    pub value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct CostYaml {
    pub ticks: Option<Spanned<ExprYaml>>,
    #[serde(default)]
    pub meters: Counts,
    #[serde(default)]
    pub items: Counts,
}

/// Counts by key under a cost. A plain map, so each count keeps its line.
pub(in crate::canon) type Counts = std::collections::BTreeMap<String, Spanned<ExprYaml>>;

/// The start of an action. Its cost is the rule's `cost`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct StartYaml {
    #[serde(default)]
    pub counts_as_act: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct FinishYaml {
    pub cost: Option<CostYaml>,
    #[serde(default)]
    pub counts_as_act: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct CheckYaml {
    pub total: Spanned<ExprYaml>,
    pub success_at: Spanned<ExprYaml>,
    pub partial_at: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(in crate::canon) enum ExprYaml {
    Literal(i64),
    Text(String),
}

/// One effect under a dash; its one key names the effect.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(in crate::canon) enum EffectYaml {
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
pub(in crate::canon) struct MeterEffectYaml {
    pub of: Spanned<String>,
    pub key: String,
    pub add: Option<Spanned<ExprYaml>>,
    pub set: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct FlagEffectYaml {
    pub of: Spanned<String>,
    pub key: String,
    pub value: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct ModifierEffectYaml {
    pub to: Spanned<String>,
    pub id: String,
    pub add: Spanned<ExprYaml>,
    /// How long this application lasts, 1 or more.
    pub ticks: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct LocationYaml {
    pub to: Option<String>,
    pub via: Option<String>,
    #[serde(default)]
    pub teleport: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct NoteYaml {
    pub id: String,
    #[serde(default)]
    pub to_ids: Vec<ActorId>,
    pub place_id: Option<PlaceId>,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct NarrateYaml {
    pub to: Option<String>,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct MemoryYaml {
    pub to: Spanned<String>,
    pub text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct ItemEffectYaml {
    pub of: Spanned<String>,
    pub id: String,
    pub add: Spanned<ExprYaml>,
    pub max: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct WieldYaml {
    pub subject: Spanned<String>,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct UnwieldYaml {
    pub subject: Spanned<String>,
}

/// A nested `check:`: `by` (`self` when left out) rolls `total` against `success_at`, and the
/// outcome picks one of the lists, any of which may be left out.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::canon) struct CheckEffectYaml {
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
