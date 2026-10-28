//! Typed YAML for a rule. Serde picks the effect and condition variants.
//! Kind names are enum variants, not strings compared in this module.

use std::fmt;

use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::Deserialize;
use serde_saphyr::Spanned;

use crate::canon::source::{serde_spot, At, Fault, Problem, Source};
use crate::diagnostic::{schema, Diagnostic};
use crate::rule::{
    parse_expr, stamp_expr, Arith, ChangeWatch, Cond, Cost, EffectTemplate, Expr, LocationTo,
    NarrateTo, Op, Owner, ResistSpec, RollSpec, Rule, Spot, TargetKind, WatchWho, When, Who,
};

const REPLACED: &str = "rules/moves.yaml was replaced by rules/verbs.yaml";

pub(crate) fn replaced_moves_file() -> Diagnostic {
    schema(
        "rules/moves.yaml",
        REPLACED,
        "delete rules/moves.yaml and write the rules in rules/verbs.yaml or in the owner's file",
    )
    .with_path("schema")
    .at_line(1, Some(1))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerbsFile {
    #[serde(rename = "schema")]
    pub _schema: String,
    #[serde(default)]
    pub rules: Vec<RuleEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleEntry {
    pub id: Spanned<String>,
    pub when: Spanned<WhenYaml>,
    #[serde(rename = "if", default)]
    pub if_: Option<IfYaml>,
    #[serde(default)]
    pub cost: Option<CostYaml>,
    #[serde(default)]
    pub roll: Option<RollYaml>,
    #[serde(rename = "do", default)]
    pub do_: Vec<EffectYaml>,
    #[serde(default)]
    pub success: Vec<EffectYaml>,
    #[serde(default)]
    pub partial: Vec<EffectYaml>,
    #[serde(default)]
    pub failure: Vec<EffectYaml>,
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
    pub who: Option<String>,
    pub meter: Option<String>,
    pub crosses: Option<String>,
    pub flag: Option<String>,
    pub becomes: Option<bool>,
    pub clock: Option<String>,
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
        struct Visit;

        impl<'de> Visitor<'de> for Visit {
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

        deserializer.deserialize_any(Visit)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CondYaml {
    pub has_item: Option<HasItemYaml>,
    pub meter: Option<MeterCondYaml>,
    pub flag: Option<FlagCondYaml>,
    pub at: Option<AtYaml>,
    pub knows: Option<KnowsYaml>,
    pub clock: Option<ClockCondYaml>,
    pub hour: Option<HourYaml>,
    pub humans: Option<HumansYaml>,
    pub note_exists: Option<String>,
    pub wielding: Option<WieldingYaml>,
    pub all: Option<Vec<CondYaml>>,
    pub any: Option<Vec<CondYaml>>,
    pub not: Option<Box<CondYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HasItemYaml {
    pub who: Spanned<String>,
    pub id: String,
    #[serde(default = "one_count")]
    pub count: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WieldingYaml {
    pub who: Spanned<String>,
    pub id: String,
}

fn one_count() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterCondYaml {
    pub who: Spanned<String>,
    pub key: String,
    pub op: String,
    pub value: Spanned<NumberYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagCondYaml {
    pub who: Spanned<String>,
    pub key: String,
    #[serde(rename = "is")]
    pub is_: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AtYaml {
    pub who: Spanned<String>,
    pub place: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowsYaml {
    pub who: Spanned<String>,
    pub fact: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockCondYaml {
    pub id: String,
    pub op: String,
    pub value: Spanned<NumberYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HourYaml {
    pub from: i64,
    pub to: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HumansYaml {
    pub op: String,
    pub value: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostYaml {
    pub time: Option<String>,
    #[serde(default)]
    pub meter: MeterCost,
    #[serde(default)]
    pub items: ItemCost,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
pub enum MeterCost {
    Map(std::collections::BTreeMap<String, Spanned<NumberYaml>>),
    #[default]
    None,
}

#[derive(Deserialize, Default)]
#[serde(untagged)]
pub enum ItemCost {
    Map(std::collections::BTreeMap<String, Spanned<NumberYaml>>),
    #[default]
    None,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollYaml {
    pub check: String,
    pub target: Spanned<NumberYaml>,
    #[serde(default)]
    pub partial_at: Option<Spanned<NumberYaml>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum NumberYaml {
    Literal(i64),
    Text(String),
    Plus(MeterPlusYaml),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterPlusYaml {
    pub meter: MeterWhoYaml,
    #[serde(default)]
    pub plus: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterWhoYaml {
    pub who: Spanned<String>,
    pub key: Spanned<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectYaml {
    pub meter: Option<MeterEffectYaml>,
    pub flag: Option<FlagEffectYaml>,
    pub location: Option<LocationYaml>,
    pub note: Option<NoteYaml>,
    pub narrate: Option<NarrateYaml>,
    pub clock: Option<ClockEffectYaml>,
    pub memory: Option<MemoryYaml>,
    pub item: Option<ItemEffectYaml>,
    pub reveal: Option<RevealYaml>,
    pub wield: Option<WieldYaml>,
    pub unwield: Option<UnwieldYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterEffectYaml {
    pub who: Spanned<String>,
    pub key: String,
    pub add: Option<Spanned<NumberYaml>>,
    pub set: Option<Spanned<NumberYaml>>,
    #[serde(default)]
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlagEffectYaml {
    pub who: Spanned<String>,
    pub key: String,
    pub value: bool,
    #[serde(default)]
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocationYaml {
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub via: Option<String>,
    #[serde(default)]
    pub teleport: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoteYaml {
    pub id: String,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub fact_ids: Vec<String>,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NarrateYaml {
    pub to: String,
    pub text: String,
    #[serde(default)]
    pub fact_ids: Vec<String>,
    #[serde(default)]
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockEffectYaml {
    pub id: String,
    #[serde(default)]
    pub add: Option<i64>,
    #[serde(default)]
    pub start: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryYaml {
    pub who: Spanned<String>,
    #[serde(default)]
    pub fact_ids: Vec<String>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemEffectYaml {
    pub who: Spanned<String>,
    pub id: String,
    pub add: i64,
    #[serde(default)]
    pub max: Option<i64>,
    #[serde(default)]
    pub resist: Option<ResistYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevealYaml {
    pub who: Spanned<String>,
    pub fact: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WieldYaml {
    pub who: Spanned<String>,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnwieldYaml {
    pub who: Spanned<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ResistYaml {
    Same(String),
    Roll(Box<ResistRollYaml>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResistRollYaml {
    #[serde(default)]
    pub who: Option<Spanned<String>>,
    pub check: String,
    pub target: Spanned<NumberYaml>,
    #[serde(default)]
    pub resisted: Vec<EffectYaml>,
}

pub(crate) fn compile(
    file: &str,
    entries: Vec<RuleEntry>,
    owner: &Owner,
) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let mut diags = Vec::new();
    let mut rules = Vec::new();
    let mut verbs: Vec<(String, String)> = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        match compile_one(file, index, entry, owner) {
            Ok(rule) => {
                if let When::Verb { verb, .. } = &rule.when {
                    if verbs.iter().any(|(name, _)| name == verb) {
                        diags.push(schema(
                            file,
                            &format!("two verb rules for '{verb}' at the same level"),
                            "keep one rule for that verb in this file",
                        ));
                    }
                    verbs.push((verb.clone(), rule.id.clone()));
                }
                if rules.iter().any(|earlier: &Rule| earlier.id == rule.id) {
                    diags.push(schema(
                        file,
                        &format!("rule '{}' is listed twice", rule.local_id),
                        "keep one rule with this id",
                    ));
                }
                rules.push(rule);
            }
            Err(more) => diags.extend(more),
        }
    }
    if diags.is_empty() {
        Ok(rules)
    } else {
        Err(diags)
    }
}

fn compile_one(
    file: &str,
    index: usize,
    entry: RuleEntry,
    owner: &Owner,
) -> Result<Rule, Vec<Diagnostic>> {
    let source = Source::named(file);
    let mut diags = Vec::new();
    let local = entry.id.value.clone();
    if local.is_empty() || local == "say" {
        diags.push(
            Problem {
                source: &source,
                path: &format!("rules[{index}].id"),
                at: At::Spot(serde_spot(&entry.id)),
                problem: "a rule id is empty or is say".into(),
                hint: "set id to a short name; say is speech",
            }
            .schema(),
        );
    }
    let when = match when_of(&entry.when.value) {
        Ok(when) => Some(when),
        Err(problem) => {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("rules[{index}].when"),
                    at: At::Spot(serde_spot(&entry.when)),
                    problem,
                    hint: "set when to habit, a verb, or a change",
                }
                .schema(),
            );
            None
        }
    };
    let if_ = match entry.if_.as_ref().map(if_of).transpose() {
        Ok(list) => list.unwrap_or_default(),
        Err(fault) => {
            diags.push(
                Problem {
                    source: &source,
                    path: "",
                    at: At::Spot(fault.spot),
                    problem: fault.message,
                    hint: "put one condition under each dash; use any: or not: to combine",
                }
                .schema(),
            );
            Vec::new()
        }
    };
    let cost = match entry.cost.as_ref().map(cost_of).transpose() {
        Ok(cost) => cost,
        Err(fault) => {
            diags.push(
                Problem {
                    source: &source,
                    path: "",
                    at: At::Spot(fault.spot),
                    problem: fault.message,
                    hint: "write time as 30s, 10m, 2h, or 1d",
                }
                .schema(),
            );
            None
        }
    };
    let roll = match entry.roll.as_ref().map(roll_of).transpose() {
        Ok(roll) => roll,
        Err(fault) => {
            diags.push(
                Problem {
                    source: &source,
                    path: "",
                    at: At::Spot(fault.spot),
                    problem: fault.message,
                    hint: "name a check and a target",
                }
                .schema(),
            );
            None
        }
    };
    let do_ = effects_of(file, &entry.do_)?;
    let success = effects_of(file, &entry.success)?;
    let partial = effects_of(file, &entry.partial)?;
    let failure = effects_of(file, &entry.failure)?;
    if let Some(when) = &when {
        if matches!(when, When::Change(_)) && cost.is_some() {
            diags.push(schema(
                file,
                "a change rule has a cost",
                "remove cost; a change is a consequence",
            ));
        }
        if roll.is_none() && (!success.is_empty() || !partial.is_empty() || !failure.is_empty()) {
            diags.push(schema(
                file,
                &format!("rule '{local}' lists an outcome without a roll"),
                "add roll or move those effects to do",
            ));
        }
        if roll.is_some() && success.is_empty() && (!partial.is_empty() || !failure.is_empty()) {
            diags.push(schema(
                file,
                &format!("rule '{local}' rolls without a success list"),
                "add success",
            ));
        }
        match &roll {
            Some(spec) if spec.partial_at.is_some() && partial.is_empty() => diags.push(schema(
                file,
                &format!("rule '{local}' sets partial_at without a partial list"),
                "add a partial list or remove partial_at",
            )),
            Some(spec) if spec.partial_at.is_none() && !partial.is_empty() => diags.push(schema(
                file,
                &format!("rule '{local}' lists partial without partial_at"),
                "set partial_at under roll",
            )),
            Some(spec)
                if spec.partial_at.as_ref().is_some_and(|band| {
                    band.constant()
                        .zip(spec.target.constant())
                        .is_some_and(|(band, target)| band >= target)
                }) =>
            {
                diags.push(schema(
                    file,
                    &format!("rule '{local}' sets partial_at at or above the target"),
                    "set partial_at strictly below the target",
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
        id: owner.id(&local),
        local_id: local,
        owner: owner.clone(),
        when,
        if_,
        roll,
        cost,
        do_,
        success,
        partial,
        failure,
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
                if verb.is_empty() || verb == "say" {
                    return Err("verb is empty or is say".to_string());
                }
                let target = match map.target.as_deref() {
                    None | Some("none") => TargetKind::None,
                    Some("actor") | Some("actor_here") => TargetKind::Actor,
                    Some("place") => TargetKind::Place,
                    Some("exit") => TargetKind::Exit,
                    Some("item") | Some("item_here") | Some("item_held") => TargetKind::Item,
                    Some(other) => return Err(format!("target '{other}' is not a target kind")),
                };
                Ok(When::Verb {
                    verb: verb.clone(),
                    target,
                })
            }
            (None, Some(change)) => change_of(change),
            _ => Err("when needs one of verb or change".to_string()),
        },
    }
}

fn change_of(yaml: &ChangeYaml) -> Result<When, String> {
    let who = match yaml.who.as_deref() {
        None | Some("any") => WatchWho::Any,
        Some("pc") => WatchWho::Pc,
        Some("npc") => WatchWho::Npc,
        Some(actor) => WatchWho::Actor(actor.to_string()),
    };
    if let Some(place) = &yaml.arrive {
        return Ok(When::Change(ChangeWatch::Arrive {
            place: place.clone(),
            who,
        }));
    }
    if let Some(place) = &yaml.leave {
        return Ok(When::Change(ChangeWatch::Leave {
            place: place.clone(),
            who,
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
    if let Some(id) = &yaml.clock {
        return Ok(When::Change(ChangeWatch::Clock { id: id.clone() }));
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
    Err("a change needs arrive, leave, meter, flag, clock, item, or knows".to_string())
}

fn if_of(yaml: &IfYaml) -> Result<Vec<Cond>, Fault> {
    match yaml {
        IfYaml::List(list) => list.iter().map(cond_of).collect(),
        IfYaml::One(one) => Ok(vec![cond_of(one)?]),
    }
}

pub(crate) fn cond_of(yaml: &CondYaml) -> Result<Cond, Fault> {
    let mut found = 0u8;
    let mut cond = None;
    if let Some(item) = &yaml.has_item {
        found += 1;
        cond = Some(Cond::HasItem {
            who: who_of(&item.who)?,
            id: item.id.clone(),
            count: item.count,
        });
    }
    if let Some(meter) = &yaml.meter {
        found += 1;
        cond = Some(Cond::Meter {
            who: who_of(&meter.who)?,
            key: meter.key.clone(),
            op: op_of(&meter.op)?,
            value: still_number(&meter.value)?,
        });
    }
    if let Some(flag) = &yaml.flag {
        found += 1;
        cond = Some(Cond::Flag {
            who: who_of(&flag.who)?,
            key: flag.key.clone(),
            is: flag.is_,
        });
    }
    if let Some(at) = &yaml.at {
        found += 1;
        cond = Some(Cond::At {
            who: who_of(&at.who)?,
            place: at.place.clone(),
        });
    }
    if let Some(knows) = &yaml.knows {
        found += 1;
        cond = Some(Cond::Knows {
            who: who_of(&knows.who)?,
            fact: knows.fact.clone(),
        });
    }
    if let Some(clock) = &yaml.clock {
        found += 1;
        cond = Some(Cond::Clock {
            id: clock.id.clone(),
            op: op_of(&clock.op)?,
            value: still_number(&clock.value)?,
        });
    }
    if let Some(hour) = &yaml.hour {
        found += 1;
        if !(0..24).contains(&hour.from) || !(0..24).contains(&hour.to) || hour.from == hour.to {
            return Err("hour needs from and to in 0..24, and they must differ".into());
        }
        cond = Some(Cond::Hour {
            from: hour.from,
            to: hour.to,
        });
    }
    if let Some(humans) = &yaml.humans {
        found += 1;
        cond = Some(Cond::Humans {
            op: op_of(&humans.op)?,
            value: humans.value,
        });
    }
    if let Some(id) = &yaml.note_exists {
        found += 1;
        cond = Some(Cond::NoteExists(id.clone()));
    }
    if let Some(wielding) = &yaml.wielding {
        found += 1;
        cond = Some(Cond::Wielding {
            who: who_of(&wielding.who)?,
            id: wielding.id.clone(),
        });
    }
    if let Some(all) = &yaml.all {
        found += 1;
        cond = Some(Cond::All(
            all.iter().map(cond_of).collect::<Result<_, _>>()?,
        ));
    }
    if let Some(any) = &yaml.any {
        found += 1;
        cond = Some(Cond::Any(
            any.iter().map(cond_of).collect::<Result<_, _>>()?,
        ));
    }
    if let Some(not) = &yaml.not {
        found += 1;
        cond = Some(Cond::Not(Box::new(cond_of(not)?)));
    }
    if found == 1 {
        return Ok(cond.expect("one condition"));
    }
    Err("two conditions under one dash".into())
}

pub(crate) fn cost_of(yaml: &CostYaml) -> Result<Cost, Fault> {
    let minutes = match &yaml.time {
        Some(text) => duration_minutes(text)?,
        None => 0,
    };
    let meters = match &yaml.meter {
        MeterCost::Map(map) => map
            .iter()
            .map(|(key, n)| Ok((key.clone(), number_of(n)?)))
            .collect::<Result<Vec<_>, Fault>>()?,
        MeterCost::None => Vec::new(),
    };
    let items = match &yaml.items {
        ItemCost::Map(map) => map
            .iter()
            .map(|(key, n)| Ok((key.clone(), number_of(n)?)))
            .collect::<Result<Vec<_>, Fault>>()?,
        ItemCost::None => Vec::new(),
    };
    Ok(Cost {
        minutes,
        meters,
        items,
    })
}

pub(crate) fn duration_minutes(text: &str) -> Result<u64, String> {
    let (count, unit) = text
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_digit())
        .map(|(index, _)| text.split_at(index))
        .ok_or_else(|| format!("time '{text}' is not a duration"))?;
    let count: u64 = count
        .parse()
        .map_err(|_| format!("time '{text}' is not a duration"))?;
    let minutes = match unit {
        "s" => count.div_ceil(60),
        "m" => count,
        "h" => count.saturating_mul(60),
        "d" => count.saturating_mul(24 * 60),
        _ => return Err(format!("time '{text}' is not 30s, 10m, 2h, or 1d")),
    };
    Ok(minutes)
}

fn roll_of(yaml: &RollYaml) -> Result<RollSpec, Fault> {
    if yaml.check.is_empty() {
        return Err("roll.check is empty".into());
    }
    Ok(RollSpec {
        check: yaml.check.clone(),
        target: number_of(&yaml.target)?,
        partial_at: yaml.partial_at.as_ref().map(number_of).transpose()?,
    })
}

fn still_number(yaml: &Spanned<NumberYaml>) -> Result<Expr, Fault> {
    let expr = number_of(yaml)?;
    if expr.needs_dice() {
        return Err(Fault {
            message: "a condition cannot roll dice".into(),
            spot: serde_spot(yaml),
        });
    }
    Ok(expr)
}

fn number_of(yaml: &Spanned<NumberYaml>) -> Result<Expr, Fault> {
    let spot = serde_spot(yaml);
    match &yaml.value {
        NumberYaml::Literal(n) => Ok(Expr::Num(*n)),
        NumberYaml::Text(text) => {
            let mut expr = parse_expr(text).map_err(|message| Fault { message, spot })?;
            if let Some(Spot { line, column }) = spot {
                stamp_expr(&mut expr, line, column);
            }
            Ok(expr)
        }
        NumberYaml::Plus(plus) => {
            let meter = Expr::Meter {
                who: who_of(&plus.meter.who)?,
                key: plus.meter.key.value.clone(),
                spot: serde_spot(&plus.meter.key).or(spot),
            };
            if plus.plus == 0 {
                Ok(meter)
            } else {
                Ok(Expr::Bin {
                    op: Arith::Add,
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
                return Err(vec![Problem {
                    source: &Source::named(file),
                    path: "",
                    at: At::Spot(fault.spot),
                    problem: fault.message,
                    hint: "one effect key under each dash",
                }
                .schema()]);
            }
        }
    }
    Ok(out)
}

fn effect_of(yaml: &EffectYaml) -> Result<EffectTemplate, Fault> {
    let mut found = 0u8;
    let mut effect = None;
    if let Some(meter) = &yaml.meter {
        found += 1;
        if meter.add.is_some() == meter.set.is_some() {
            return Err("a meter effect needs add or set, not both".into());
        }
        effect = Some(EffectTemplate::Meter {
            who: who_of(&meter.who)?,
            key: meter.key.clone(),
            add: meter.add.as_ref().map(number_of).transpose()?,
            set: meter.set.as_ref().map(number_of).transpose()?,
            resist: resist_of(meter.resist.as_ref())?,
        });
    }
    if let Some(flag) = &yaml.flag {
        found += 1;
        if flag.key == "busy" {
            return Err("flag key busy is not a flag".into());
        }
        effect = Some(EffectTemplate::Flag {
            who: who_of(&flag.who)?,
            key: flag.key.clone(),
            value: flag.value,
            resist: resist_of(flag.resist.as_ref())?,
        });
    }
    if let Some(location) = &yaml.location {
        found += 1;
        let to = match (&location.to, &location.via) {
            (Some(word), None) if word == "target" => LocationTo::Target,
            (Some(place), None) => LocationTo::Place(place.clone()),
            (None, Some(via)) => LocationTo::Via(via.clone()),
            _ => return Err("location needs to or via".into()),
        };
        effect = Some(EffectTemplate::Location {
            to,
            teleport: location.teleport,
        });
    }
    if let Some(note) = &yaml.note {
        found += 1;
        if note.id.is_empty() || note.text.is_empty() || note.to.is_empty() {
            return Err("a note needs id, text, and to".into());
        }
        effect = Some(EffectTemplate::Note {
            id: note.id.clone(),
            to: note.to.clone(),
            place: note.place.clone(),
            fact_ids: note.fact_ids.clone(),
            text: note.text.clone(),
        });
    }
    if let Some(narrate) = &yaml.narrate {
        found += 1;
        let to = match narrate.to.as_str() {
            "here" => NarrateTo::Here,
            "self" => NarrateTo::Self_,
            other => NarrateTo::Actors(vec![other.to_string()]),
        };
        effect = Some(EffectTemplate::Narrate {
            to,
            text: narrate.text.clone(),
            fact_ids: narrate.fact_ids.clone(),
            resist: resist_of(narrate.resist.as_ref())?,
        });
    }
    if let Some(clock) = &yaml.clock {
        found += 1;
        if clock.add.is_none() && !clock.start {
            return Err("a clock effect needs add or start".into());
        }
        effect = Some(EffectTemplate::Clock {
            id: clock.id.clone(),
            add: clock.add,
            start: clock.start,
        });
    }
    if let Some(memory) = &yaml.memory {
        found += 1;
        effect = Some(EffectTemplate::Memory {
            who: who_of(&memory.who)?,
            fact_ids: memory.fact_ids.clone(),
            text: memory.text.clone(),
        });
    }
    if let Some(item) = &yaml.item {
        found += 1;
        effect = Some(EffectTemplate::Item {
            who: who_of(&item.who)?,
            id: item.id.clone(),
            add: item.add,
            max: item.max,
            resist: resist_of(item.resist.as_ref())?,
        });
    }
    if let Some(reveal) = &yaml.reveal {
        found += 1;
        effect = Some(EffectTemplate::Reveal {
            who: who_of(&reveal.who)?,
            fact: reveal.fact.clone(),
        });
    }
    if let Some(wield) = &yaml.wield {
        found += 1;
        effect = Some(EffectTemplate::Wield {
            who: who_of(&wield.who)?,
            id: wield.id.clone(),
        });
    }
    if let Some(unwield) = &yaml.unwield {
        found += 1;
        effect = Some(EffectTemplate::Unwield {
            who: who_of(&unwield.who)?,
        });
    }
    if found == 1 {
        return Ok(effect.expect("one effect"));
    }
    Err("one effect per dash".into())
}

fn resist_of(yaml: Option<&ResistYaml>) -> Result<Option<ResistSpec>, Fault> {
    let Some(yaml) = yaml else {
        return Ok(None);
    };
    match yaml {
        ResistYaml::Same(word) if word == "same" => Ok(Some(ResistSpec::Same)),
        ResistYaml::Same(word) => Err(format!("resist '{word}' is not same").into()),
        ResistYaml::Roll(roll) => Ok(Some(ResistSpec::Roll {
            who: match &roll.who {
                Some(who) => who_of(who)?,
                None => Who::Target,
            },
            check: roll.check.clone(),
            target: number_of(&roll.target)?,
            resisted: roll
                .resisted
                .iter()
                .map(effect_of)
                .collect::<Result<Vec<_>, _>>()?,
        })),
    }
}

fn who_of(yaml: &Spanned<String>) -> Result<Who, Fault> {
    let fault = |message: String| Fault {
        message,
        spot: serde_spot(yaml),
    };
    let word = yaml.value.as_str();
    if let Some(place) = word.strip_prefix("place:") {
        if place.is_empty() {
            return Err(fault("place: needs an id".to_string()));
        }
        return Ok(Who::Place(place.to_string()));
    }
    match word {
        "self" => Ok(Who::Self_),
        "target" => Ok(Who::Target),
        "here" => Ok(Who::Here),
        "world" => Ok(Who::World),
        "subject" => Err(fault(
            "who 'subject' is not a who; use self, or target for the move's target".to_string(),
        )),
        other if !other.is_empty() && !retired_selector(other) => Ok(Who::Actor(other.to_string())),
        other => Err(fault(format!("who '{other}' is not a who"))),
    }
}

fn retired_selector(word: &str) -> bool {
    matches!(
        word,
        "token"
            | "first_rate"
            | "rate"
            | "first_drift"
            | "toward_target"
            | "first_known"
            | "first_listener"
            | "keep"
            | "first_due_note"
            | "first_owned"
    )
}

fn op_of(word: &str) -> Result<Op, String> {
    Op::parse(word).ok_or_else(|| format!("op '{word}' is not <, <=, ==, >=, or >"))
}

fn cross_of(text: &str) -> Result<(Op, i64), String> {
    let text = text.trim();
    let (op, rest) = if let Some(rest) = text.strip_prefix("<=") {
        (Op::Le, rest)
    } else if let Some(rest) = text.strip_prefix(">=") {
        (Op::Ge, rest)
    } else if let Some(rest) = text.strip_prefix("==") {
        (Op::Eq, rest)
    } else if let Some(rest) = text.strip_prefix('<') {
        (Op::Lt, rest)
    } else if let Some(rest) = text.strip_prefix('>') {
        (Op::Gt, rest)
    } else {
        return Err(format!("crosses '{text}' needs an operator and a number"));
    };
    let bound = rest
        .trim()
        .parse::<i64>()
        .map_err(|_| format!("crosses '{text}' needs an operator and a number"))?;
    Ok((op, bound))
}
