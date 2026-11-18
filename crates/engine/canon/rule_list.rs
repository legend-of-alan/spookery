//! Typed YAML for a rule. Serde picks the effect and condition variants.
//! Kind names are enum variants, not strings compared in this module.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_saphyr::Spanned;
use spookery_protocol::{is_built_in, Seat};

use crate::canon::source::{
    load_tagged, spot_of, Diags, Fault, FileText, Locator, Problem, VERBS_SCHEMA,
};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::ecs::id::{ActorId, PlaceId};
use crate::rules::expr::{stamp_spots, BinOp, Expr};
use crate::rules::grammar;
use crate::rules::rule::{
    ChangeWatch, Cond, Cost, EffectTemplate, LineColumn, LocationTo, NarrateTo, Op, Owner, Phase,
    ResistSpec, RollSpec, Rule, TargetKind, When,
};

const MOVES_REPLACED: &str = "rules/moves.yaml was replaced by rules/verbs.yaml";

pub(super) fn replaced_moves_file() -> Diagnostic {
    schema_error(
        "rules/moves.yaml",
        MOVES_REPLACED,
        "delete rules/moves.yaml and write the rules in rules/verbs.yaml or in the owner's file",
    )
    .at_path("schema")
    .at_line(1, Some(1))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbsDoc {
    #[serde(rename = "schema")]
    pub _schema: String,
    #[serde(default)]
    pub rules: Vec<RuleYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleYaml {
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
    pub roll: Option<RollYaml>,
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
pub enum WhenYaml {
    Word(String),
    Map(Box<WhenMap>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WhenMap {
    pub verb: Option<String>,
    pub target: Option<String>,
    pub change: Option<ChangeYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeYaml {
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
    pub knows: Option<String>,
}

pub enum IfYaml {
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

/// One condition under a dash; its one key names the condition.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum CondYaml {
    HasItem(HasItemYaml),
    Meter(MeterCondYaml),
    Flag(FlagCondYaml),
    At(AtYaml),
    Knows(KnowsYaml),
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
pub struct HasItemYaml {
    pub subject: Spanned<String>,
    pub id: String,
    #[serde(default = "default_count")]
    pub count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WieldingYaml {
    pub subject: Spanned<String>,
    pub id: String,
}

fn default_count() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterCondYaml {
    pub of: Spanned<String>,
    pub id: String,
    pub op: String,
    pub value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagCondYaml {
    pub of: Spanned<String>,
    pub key: String,
    #[serde(rename = "is")]
    pub is_: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtYaml {
    pub subject: Spanned<String>,
    pub place: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowsYaml {
    pub subject: Spanned<String>,
    pub fact: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CycleYaml {
    pub id: String,
    pub from: u64,
    pub to: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayersYaml {
    pub op: String,
    pub value: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostYaml {
    #[serde(default)]
    pub ticks: u64,
    #[serde(default)]
    pub meters: Counts,
    #[serde(default)]
    pub items: Counts,
}

/// The start of an action. Its cost is the rule's `cost`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartYaml {
    #[serde(default)]
    pub counts_as_act: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinishYaml {
    pub cost: Option<CostYaml>,
    #[serde(default)]
    pub counts_as_act: bool,
}

/// Counts by key under a cost. The unit variant keeps a scalar's refusal generic.
#[derive(Deserialize, Default)]
#[serde(untagged)]
pub enum Counts {
    Map(std::collections::BTreeMap<String, Spanned<ExprYaml>>),
    #[default]
    None,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollYaml {
    pub check: String,
    pub threshold: Spanned<ExprYaml>,
    pub partial_at: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ExprYaml {
    Literal(i64),
    Text(String),
    Plus(MeterPlusYaml),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterPlusYaml {
    pub meter: MeterOfYaml,
    #[serde(default)]
    pub plus: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterOfYaml {
    pub of: Spanned<String>,
    pub key: Spanned<String>,
}

/// One effect under a dash; its one key names the effect.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EffectYaml {
    Meter(MeterEffectYaml),
    Flag(FlagEffectYaml),
    Modifier(ModifierEffectYaml),
    Location(LocationYaml),
    Note(NoteYaml),
    Narrate(NarrateYaml),
    Memory(MemoryYaml),
    Item(ItemEffectYaml),
    Reveal(RevealYaml),
    Wield(WieldYaml),
    Unwield(UnwieldYaml),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterEffectYaml {
    pub of: Spanned<String>,
    pub key: String,
    pub add: Option<Spanned<ExprYaml>>,
    pub set: Option<Spanned<ExprYaml>>,
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagEffectYaml {
    pub of: Spanned<String>,
    pub key: String,
    pub value: bool,
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifierEffectYaml {
    pub to: Spanned<String>,
    pub id: String,
    pub add: Spanned<ExprYaml>,
    /// How long this application lasts, 1 or more.
    pub ticks: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationYaml {
    pub to: Option<String>,
    pub via: Option<String>,
    #[serde(default)]
    pub teleport: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteYaml {
    pub id: String,
    #[serde(default)]
    pub to_ids: Vec<ActorId>,
    pub place_id: Option<PlaceId>,
    #[serde(default)]
    pub fact_ids: Vec<String>,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrateYaml {
    pub to: Option<String>,
    pub text: String,
    #[serde(default)]
    pub fact_ids: Vec<String>,
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryYaml {
    pub to: Spanned<String>,
    #[serde(default)]
    pub fact_ids: Vec<String>,
    pub text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemEffectYaml {
    pub of: Spanned<String>,
    pub id: String,
    pub add: i64,
    pub max: Option<i64>,
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevealYaml {
    pub to: Spanned<String>,
    pub fact: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WieldYaml {
    pub subject: Spanned<String>,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnwieldYaml {
    pub subject: Spanned<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ResistYaml {
    Word(String),
    Roll(Box<ResistRollYaml>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResistRollYaml {
    pub subject: Option<Spanned<String>>,
    pub check: String,
    pub threshold: Spanned<ExprYaml>,
    #[serde(default)]
    pub resisted: Vec<EffectYaml>,
}

pub(crate) fn compile(
    file: &str,
    entries: Vec<RuleYaml>,
    owner: &Owner,
) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let source = FileText::named(file);
    let mut diags = Diags::default();
    let mut rules: Vec<Rule> = Vec::new();
    let mut verb_ids = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(rule) = diags.keep(compile_one(file, index, entry, owner)) else {
            continue;
        };
        if let When::Verb { verb_id, .. } = &rule.when {
            let hint = "keep one rule for that verb in this file";
            let twice = ("two verb rules for", "at the same level", hint);
            let taken = verb_ids.contains(verb_id);
            diags.extend(
                taken.then(|| source.refuse_duplicate(verb_id, "", Locator::Nowhere, twice)),
            );
            verb_ids.push(verb_id.clone());
        }
        let twice = ("rule", "is listed twice", "keep one rule with this id");
        let taken = rules.iter().any(|earlier| earlier.id == rule.id);
        diags.extend(
            taken.then(|| source.refuse_duplicate(&rule.local_id, "", Locator::Nowhere, twice)),
        );
        rules.push(rule);
    }
    diags.into_result(rules)
}

fn compile_one(
    file: &str,
    index: usize,
    entry: RuleYaml,
    owner: &Owner,
) -> Result<Rule, Vec<Diagnostic>> {
    let source = FileText::named(file);
    let mut diags = Vec::new();
    let local = entry.id.value.clone();
    if local.is_empty() || local == "say" {
        diags.push(
            Problem {
                source: &source,
                path: &format!("rules[{index}].id"),
                at: Locator::Spot(spot_of(&entry.id)),
                message: "a rule id is empty or is say".into(),
                hint: "set id to a short rule id; say is speech",
            }
            .into_diagnostic(),
        );
    }
    let when = match when_of(&entry.when.value) {
        Ok(when) => Some(when),
        Err(problem) => {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("rules[{index}].when"),
                    at: Locator::Spot(spot_of(&entry.when)),
                    message: problem,
                    hint: "set when to habit, a verb, or a change",
                }
                .into_diagnostic(),
            );
            None
        }
    };
    let if_ = match entry.if_.as_ref().map(if_of).transpose() {
        Ok(list) => list.unwrap_or_default(),
        Err(fault) => {
            diags.push(source.fault("", fault));
            Vec::new()
        }
    };
    let cost = match entry.cost.as_ref().map(cost_of).transpose() {
        Ok(cost) => cost,
        Err(fault) => {
            diags.push(source.fault("", fault));
            None
        }
    };
    let finish_cost = entry
        .finish
        .as_ref()
        .and_then(|finish| finish.cost.as_ref());
    let finish_cost = match finish_cost.map(cost_of).transpose() {
        Ok(Some(cost)) if cost.ticks > 0 => {
            diags.push(schema_error(
                file,
                &format!("rule '{local}' has ticks in its finish cost"),
                "put the action's ticks in cost; the finish takes no time",
            ));
            None
        }
        Ok(cost) => cost,
        Err(fault) => {
            diags.push(source.fault("", fault));
            None
        }
    };
    let roll = match entry.roll.as_ref().map(roll_of).transpose() {
        Ok(roll) => roll,
        Err(fault) => {
            diags.push(source.fault("", fault));
            None
        }
    };
    let do_ = effects_of(file, &entry.do_)?;
    let success = effects_of(file, &entry.success)?;
    let partial = effects_of(file, &entry.partial)?;
    let miss = effects_of(file, &entry.miss)?;
    if let Some(when) = &when {
        let timed = cost.is_some()
            || entry.start.is_some()
            || entry.finish.is_some()
            || entry.cooldown > 0
            || entry.cancellable;
        if matches!(when, When::Change(_)) && timed {
            diags.push(schema_error(
                file,
                "a change rule has a cost, start, finish, cooldown, or cancellable",
                "remove them; a change is a consequence, not an action",
            ));
        }
        if roll.is_none() && (!success.is_empty() || !partial.is_empty() || !miss.is_empty()) {
            diags.push(schema_error(
                file,
                &format!("rule '{local}' lists an outcome without a roll"),
                "add roll or move those effects to do",
            ));
        }
        if roll.is_some() && success.is_empty() && (!partial.is_empty() || !miss.is_empty()) {
            diags.push(schema_error(
                file,
                &format!("rule '{local}' rolls without a success list"),
                "add success",
            ));
        }
        match &roll {
            Some(spec) if spec.partial_at.is_some() && partial.is_empty() => {
                diags.push(schema_error(
                    file,
                    &format!("rule '{local}' sets partial_at without a partial list"),
                    "add a partial list or remove partial_at",
                ))
            }
            Some(spec) if spec.partial_at.is_none() && !partial.is_empty() => {
                diags.push(schema_error(
                    file,
                    &format!("rule '{local}' lists partial without partial_at"),
                    "set partial_at under roll",
                ))
            }
            Some(spec)
                if spec.partial_at.as_ref().is_some_and(|band| {
                    band.constant()
                        .zip(spec.threshold.constant())
                        .is_some_and(|(band, threshold)| band >= threshold)
                }) =>
            {
                diags.push(schema_error(
                    file,
                    &format!("rule '{local}' sets partial_at at or above the threshold"),
                    "set partial_at strictly below the threshold",
                ));
            }
            _ => {}
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    let when = when.expect("when checked");
    Ok(Rule {
        id: owner.rule_id(&local),
        local_id: local,
        owner: owner.clone(),
        when,
        if_,
        roll,
        start: Phase {
            cost,
            counts_as_act: entry
                .start
                .as_ref()
                .and_then(|start| start.counts_as_act)
                .unwrap_or(true),
        },
        finish: Phase {
            cost: finish_cost,
            counts_as_act: entry
                .finish
                .as_ref()
                .is_some_and(|finish| finish.counts_as_act),
        },
        cooldown: entry.cooldown,
        cancellable: entry.cancellable,
        do_,
        success,
        partial,
        miss,
        file: file.to_string(),
        index,
    })
}

fn when_of(yaml: &WhenYaml) -> Result<When, String> {
    match yaml {
        WhenYaml::Word(word) if word == "habit" => Ok(When::Habit),
        WhenYaml::Word(word) => Err(format!("when '{word}' is not habit")),
        WhenYaml::Map(map) => match (&map.verb, &map.change) {
            (Some(verb), None) => {
                if verb.is_empty() {
                    return Err("verb is empty".to_string());
                }
                if is_built_in(Seat::Player, verb) {
                    return Err(format!(
                        "verb '{verb}' is a built-in command; give the rule another verb"
                    ));
                }
                let target = match map.target.as_deref() {
                    None | Some("none") => TargetKind::None,
                    Some("actor") => TargetKind::Actor,
                    Some("place") => TargetKind::Place,
                    Some("exit") => TargetKind::Exit,
                    Some("item") => TargetKind::Item,
                    Some(other) => return Err(format!("target '{other}' is not a target kind")),
                };
                Ok(When::Verb {
                    verb_id: verb.clone(),
                    target,
                })
            }
            (None, Some(change)) => change_of(change),
            _ => Err("when needs one of verb or change".to_string()),
        },
    }
}

fn change_of(yaml: &ChangeYaml) -> Result<When, String> {
    let subject = match yaml.subject.as_deref() {
        None | Some("any") => grammar::Subject::Any,
        Some("pc") => grammar::Subject::Pc,
        Some("npc") => grammar::Subject::Npc,
        Some(actor) => grammar::Subject::Actor(actor.to_string()),
    };
    if let Some(place) = &yaml.arrive {
        return Ok(When::Change(ChangeWatch::Arrive {
            place: place.clone(),
            subject,
        }));
    }
    if let Some(place) = &yaml.leave {
        return Ok(When::Change(ChangeWatch::Leave {
            place: place.clone(),
            subject,
        }));
    }
    if let Some(key) = &yaml.meter {
        let crosses = yaml
            .crosses
            .as_deref()
            .ok_or_else(|| "a meter change needs crosses".to_string())?;
        let (op, bound) = cross_of(crosses)?;
        return Ok(When::Change(ChangeWatch::Meter {
            key: key.clone(),
            op,
            bound,
        }));
    }
    if let Some(key) = &yaml.flag {
        let becomes = yaml
            .becomes
            .ok_or_else(|| "a flag change needs becomes".to_string())?;
        return Ok(When::Change(ChangeWatch::Flag {
            key: key.clone(),
            becomes,
        }));
    }
    if let Some(id) = &yaml.item {
        let gained = match (yaml.gained, yaml.lost) {
            (Some(true), _) => true,
            (_, Some(true)) => false,
            _ => return Err("an item change needs gained or lost".to_string()),
        };
        return Ok(When::Change(ChangeWatch::Item {
            id: id.clone(),
            gained,
        }));
    }
    if let Some(fact) = &yaml.knows {
        return Ok(When::Change(ChangeWatch::Knows { fact: fact.clone() }));
    }
    Err("a change needs arrive, leave, meter, flag, item, or knows".to_string())
}

fn if_of(yaml: &IfYaml) -> Result<Vec<Cond>, Fault> {
    match yaml {
        IfYaml::List(list) => list.iter().map(cond_of).collect(),
        IfYaml::One(one) => Ok(vec![cond_of(one)?]),
    }
}

pub(super) fn cond_of(yaml: &CondYaml) -> Result<Cond, Fault> {
    Ok(match yaml {
        CondYaml::HasItem(item) => Cond::HasItem {
            subject: object_of(&item.subject)?,
            id: item.id.clone(),
            count: item.count,
        },
        CondYaml::Meter(meter) => Cond::Meter {
            of: object_of(&meter.of)?,
            key: meter.id.clone(),
            op: op_of(&meter.op)?,
            value: cond_value_of(&meter.value)?,
        },
        CondYaml::Flag(flag) => Cond::Flag {
            of: object_of(&flag.of)?,
            key: flag.key.clone(),
            is: flag.is_,
        },
        CondYaml::At(at) => Cond::At {
            subject: object_of(&at.subject)?,
            place: at.place.clone(),
        },
        CondYaml::Knows(knows) => Cond::Knows {
            subject: object_of(&knows.subject)?,
            fact: knows.fact.clone(),
        },
        CondYaml::Cycle(cycle) if cycle.from == cycle.to => {
            return Err("cycle needs from and to that differ".into());
        }
        CondYaml::Cycle(cycle) => Cond::Cycle {
            id: cycle.id.clone(),
            from: cycle.from,
            to: cycle.to,
        },
        CondYaml::Players(players) => Cond::Players {
            op: op_of(&players.op)?,
            value: players.value,
        },
        CondYaml::NoteExists(id) => Cond::NoteExists(id.clone()),
        CondYaml::Wielding(wielding) => Cond::Wielding {
            subject: object_of(&wielding.subject)?,
            id: wielding.id.clone(),
        },
        CondYaml::All(all) => Cond::All(all.iter().map(cond_of).collect::<Result<_, _>>()?),
        CondYaml::Any(any) => Cond::Any(any.iter().map(cond_of).collect::<Result<_, _>>()?),
        CondYaml::Not(not) => Cond::Not(Box::new(cond_of(not)?)),
    })
}

pub(super) fn cost_of(yaml: &CostYaml) -> Result<Cost, Fault> {
    Ok(Cost {
        ticks: yaml.ticks,
        meters: counts_of(&yaml.meters)?,
        items: counts_of(&yaml.items)?,
    })
}

fn counts_of(yaml: &Counts) -> Result<Vec<(String, Expr)>, Fault> {
    match yaml {
        Counts::Map(map) => map
            .iter()
            .map(|(key, n)| Ok((key.clone(), expr_of(n)?)))
            .collect(),
        Counts::None => Ok(Vec::new()),
    }
}

fn roll_of(yaml: &RollYaml) -> Result<RollSpec, Fault> {
    if yaml.check.is_empty() {
        return Err("roll.check is empty".into());
    }
    Ok(RollSpec {
        check: yaml.check.clone(),
        threshold: expr_of(&yaml.threshold)?,
        partial_at: yaml.partial_at.as_ref().map(expr_of).transpose()?,
    })
}

fn cond_value_of(yaml: &Spanned<ExprYaml>) -> Result<Expr, Fault> {
    let expr = expr_of(yaml)?;
    if expr.needs_dice() {
        return Err(Fault {
            message: "a condition cannot roll dice".into(),
            spot: spot_of(yaml),
        });
    }
    Ok(expr)
}

fn expr_of(yaml: &Spanned<ExprYaml>) -> Result<Expr, Fault> {
    let spot = spot_of(yaml);
    match &yaml.value {
        ExprYaml::Literal(n) => Ok(Expr::Num(*n)),
        ExprYaml::Text(text) => {
            let mut expr = Expr::parse(text).map_err(|message| Fault { message, spot })?;
            if let Some(LineColumn { line, column }) = spot {
                stamp_spots(&mut expr, line, column);
            }
            Ok(expr)
        }
        ExprYaml::Plus(plus) => {
            let meter = Expr::Meter {
                of: object_of(&plus.meter.of)?,
                key: plus.meter.key.value.clone(),
                spot: spot_of(&plus.meter.key).or(spot),
            };
            if plus.plus == 0 {
                Ok(meter)
            } else {
                Ok(Expr::Binary {
                    op: BinOp::Add,
                    left: Box::new(meter),
                    right: Box::new(Expr::Num(plus.plus)),
                })
            }
        }
    }
}

fn effects_of(file: &str, list: &[EffectYaml]) -> Result<Vec<EffectTemplate>, Vec<Diagnostic>> {
    let mut out = Vec::new();
    for effect in list {
        match effect_of(effect) {
            Ok(effect) => out.push(effect),
            Err(fault) => {
                return Err(vec![FileText::named(file).fault("", fault)]);
            }
        }
    }
    Ok(out)
}

fn effect_of(yaml: &EffectYaml) -> Result<EffectTemplate, Fault> {
    Ok(match yaml {
        EffectYaml::Meter(meter) if meter.add.is_some() == meter.set.is_some() => {
            return Err("a meter effect needs add or set, not both".into());
        }
        EffectYaml::Meter(meter) => EffectTemplate::Meter {
            of: object_of(&meter.of)?,
            key: meter.key.clone(),
            add: meter.add.as_ref().map(expr_of).transpose()?,
            set: meter.set.as_ref().map(expr_of).transpose()?,
            resist: resist_of(meter.resist.as_ref())?,
        },
        EffectYaml::Flag(flag) if flag.key == "busy" => {
            return Err("flag key busy is not a flag".into());
        }
        EffectYaml::Flag(flag) => EffectTemplate::Flag {
            of: object_of(&flag.of)?,
            key: flag.key.clone(),
            value: flag.value,
            resist: resist_of(flag.resist.as_ref())?,
        },
        EffectYaml::Modifier(modifier) if modifier.ticks == 0 => {
            return Err("a modifier effect has ticks below 1, so set ticks to 1 or more".into());
        }
        EffectYaml::Modifier(modifier) => EffectTemplate::Modifier {
            to: object_of(&modifier.to)?,
            id: modifier.id.clone(),
            add: expr_of(&modifier.add)?,
            ticks: modifier.ticks,
        },
        EffectYaml::Location(location) => EffectTemplate::Location {
            to: match (&location.to, &location.via) {
                (Some(word), None) if word == "target" => LocationTo::Target,
                (Some(place), None) => LocationTo::Place(place.clone()),
                (None, Some(via)) => LocationTo::Via(via.clone()),
                _ => return Err("location needs to or via".into()),
            },
            teleport: location.teleport,
        },
        EffectYaml::Note(note)
            if note.id.is_empty() || note.text.is_empty() || note.to_ids.is_empty() =>
        {
            return Err("a note needs id, text, and to_ids".into());
        }
        EffectYaml::Note(note) => EffectTemplate::Note {
            id: note.id.clone(),
            to_ids: note.to_ids.clone(),
            place_id: note.place_id.clone(),
            fact_ids: note.fact_ids.clone(),
            text: note.text.clone(),
        },
        EffectYaml::Narrate(narrate) => EffectTemplate::Narrate {
            to: match narrate.to.as_deref() {
                Some("here") => NarrateTo::Here,
                Some("self") => NarrateTo::Self_,
                Some(other) => NarrateTo::Actors(vec![other.into()]),
                None => NarrateTo::Present,
            },
            text: narrate.text.clone(),
            fact_ids: narrate.fact_ids.clone(),
            resist: resist_of(narrate.resist.as_ref())?,
        },
        EffectYaml::Memory(memory) => EffectTemplate::Memory {
            to: object_of(&memory.to)?,
            fact_ids: memory.fact_ids.clone(),
            text: memory.text.clone(),
        },
        EffectYaml::Item(item) => EffectTemplate::Item {
            of: object_of(&item.of)?,
            id: item.id.clone(),
            add: item.add,
            max: item.max,
            resist: resist_of(item.resist.as_ref())?,
        },
        EffectYaml::Reveal(reveal) => EffectTemplate::Reveal {
            to: object_of(&reveal.to)?,
            fact: reveal.fact.clone(),
        },
        EffectYaml::Wield(wield) => EffectTemplate::Wield {
            subject: object_of(&wield.subject)?,
            id: wield.id.clone(),
        },
        EffectYaml::Unwield(unwield) => EffectTemplate::Unwield {
            subject: object_of(&unwield.subject)?,
        },
    })
}

fn resist_of(yaml: Option<&ResistYaml>) -> Result<Option<ResistSpec>, Fault> {
    let Some(yaml) = yaml else {
        return Ok(None);
    };
    match yaml {
        ResistYaml::Word(word) if word == "same" => Ok(Some(ResistSpec::Same)),
        ResistYaml::Word(word) => Err(format!("resist '{word}' is not same").into()),
        ResistYaml::Roll(roll) => Ok(Some(ResistSpec::Roll {
            subject: match &roll.subject {
                Some(subject) => object_of(subject)?,
                None => grammar::Object::Target,
            },
            check: roll.check.clone(),
            threshold: expr_of(&roll.threshold)?,
            resisted: roll
                .resisted
                .iter()
                .map(effect_of)
                .collect::<Result<Vec<_>, _>>()?,
        })),
    }
}

fn object_of(yaml: &Spanned<String>) -> Result<grammar::Object, Fault> {
    grammar::Object::parse(&yaml.value).map_err(|message| Fault {
        message,
        spot: spot_of(yaml),
    })
}

fn op_of(word: &str) -> Result<Op, String> {
    Op::parse(word).ok_or_else(|| format!("op '{word}' is not <, <=, ==, >=, or >"))
}

fn cross_of(text: &str) -> Result<(Op, i64), String> {
    let text = text.trim();
    let split = text.find(|ch| !matches!(ch, '<' | '=' | '>'));
    let (op, bound) = text.split_at(split.unwrap_or(text.len()));
    Op::parse(op)
        .zip(bound.trim().parse::<i64>().ok())
        .ok_or_else(|| format!("crosses '{text}' needs an operator and a number"))
}

pub(super) fn load_verbs(rel: &str, text: &str) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let doc: VerbsDoc = load_tagged(rel, text, VERBS_SCHEMA, 0)?;
    compile(rel, doc.rules, &Owner::Verb)
}
