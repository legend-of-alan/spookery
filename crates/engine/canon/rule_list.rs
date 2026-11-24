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
use crate::rules::expr::{self, stamp_spots, Expr};
use crate::rules::grammar;
use crate::rules::rule::{
    self, ChangeWatch, Cond, Cost, EffectTemplate, LineColumn, LocationTo, NarrateTo, Op, Owner,
    Phase, Rule, TargetKind, When,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VerbsDoc {
    #[serde(rename = "schema")]
    pub _schema: String,
    #[serde(default)]
    pub rules: Vec<RuleYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleYaml {
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
    /// `let:`: each id and its number. A map keeps no order, so `compile_one` sorts the
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
pub(super) enum WhenYaml {
    Word(String),
    Map(Box<WhenMap>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WhenMap {
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

/// One condition; its one key names the condition.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub(super) enum CondYaml {
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
pub(super) struct HasItemYaml {
    pub subject: Spanned<String>,
    pub id: String,
    /// How many the subject holds at least; 1 when left out.
    pub count: Option<Spanned<ExprYaml>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WieldingYaml {
    pub subject: Spanned<String>,
    pub id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeterCondYaml {
    pub of: Spanned<String>,
    pub id: String,
    pub op: String,
    pub value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FlagCondYaml {
    pub of: Spanned<String>,
    pub key: String,
    #[serde(rename = "is")]
    pub is_: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AtYaml {
    pub subject: Spanned<String>,
    pub place: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CycleYaml {
    pub id: String,
    pub from: Spanned<ExprYaml>,
    pub to: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PlayersYaml {
    pub op: String,
    pub value: Spanned<ExprYaml>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CostYaml {
    pub ticks: Option<Spanned<ExprYaml>>,
    #[serde(default)]
    pub meters: Counts,
    #[serde(default)]
    pub items: Counts,
}

/// Counts by key under a cost. A plain map, so each count keeps its line.
pub(super) type Counts = std::collections::BTreeMap<String, Spanned<ExprYaml>>;

/// The start of an action. Its cost is the rule's `cost`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartYaml {
    #[serde(default)]
    pub counts_as_act: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FinishYaml {
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
    #[serde(default)]
    pub to_ids: Vec<ActorId>,
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
    let finish_yaml = entry
        .finish
        .as_ref()
        .and_then(|finish| finish.cost.as_ref());
    let finish_cost = match finish_yaml.map(cost_of).transpose() {
        Ok(Some(_)) if finish_yaml.is_some_and(|cost| cost.ticks.is_some()) => {
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
    let mut lets: Vec<_> = entry.lets.iter().collect();
    lets.sort_by_key(|(_, number)| (number.referenced.line(), number.referenced.column()));
    let lets = (lets.into_iter())
        .map(|(id, number)| let_of(id, number))
        .collect::<Result<Vec<_>, Fault>>();
    let lets = match lets {
        Ok(lets) => lets,
        Err(fault) => {
            diags.push(source.fault("", fault));
            Vec::new()
        }
    };
    let check = match (entry.check.as_ref())
        .map(|yaml| check_of(&yaml.total, &yaml.success_at, yaml.partial_at.as_ref()))
        .transpose()
    {
        Ok(check) => check,
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
        if check.is_none() && (!success.is_empty() || !partial.is_empty() || !miss.is_empty()) {
            diags.push(schema_error(
                file,
                &format!("rule '{local}' lists an outcome without a check"),
                "add check or move those effects to do",
            ));
        }
        if check.is_some() && success.is_empty() && (!partial.is_empty() || !miss.is_empty()) {
            diags.push(schema_error(
                file,
                &format!("rule '{local}' has a check without a success list"),
                "add success",
            ));
        }
        let mut checks: Vec<_> = check.iter().map(|check| (check, &partial[..])).collect();
        for effect in [&do_, &success, &partial, &miss].into_iter().flatten() {
            effect.visit(|effect| {
                if let EffectTemplate::Check { check, partial, .. } = effect {
                    checks.push((check, &partial[..]));
                }
            });
        }
        let band_problem = (checks.into_iter())
            .find_map(|(check, partial)| find_partial_band_problem(check, partial));
        if let Some((problem, hint)) = band_problem {
            diags.push(
                Problem {
                    source: &source,
                    path: &format!("rules[{index}].id"),
                    at: Locator::Spot(spot_of(&entry.id)),
                    message: format!("rule '{local}' {problem}"),
                    hint,
                }
                .into_diagnostic(),
            );
        }
    }
    if !diags.is_empty() {
        return Err(diags);
    }
    let when = when.expect("when checked");
    let rule = Rule {
        id: owner.rule_id(&local),
        local_id: local,
        owner: owner.clone(),
        when,
        if_,
        lets,
        check,
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
    };
    let targets = match &rule.when {
        When::Verb { target, .. } => *target,
        When::Habit | When::Change(_) => TargetKind::None,
    };
    let misplaced_target = rule.mentions().find_misplaced_target(targets);
    let misplaced = misplaced_target.map(|(key, needs)| {
        let (needs, targets) = (needs.as_str(), targets.as_str());
        (
            format!("says target in {key}, but its target is {targets}"),
            format!("give the verb target: {needs}, or name the {needs} by its id"),
        )
    });
    if let Some((problem, hint)) = misplaced.or_else(|| find_let_read_problem(&rule)) {
        return Err(vec![Problem {
            source: &source,
            path: &format!("rules[{index}].id"),
            at: Locator::Spot(spot_of(&entry.id)),
            message: format!("rule '{}' {problem}", rule.local_id),
            hint: &hint,
        }
        .into_diagnostic()]);
    }
    Ok(rule)
}

/// The first `let:` read in `rule` that fails, as the problem and the hint: an id its `let:`
/// does not hold, an id a `let:` number reads at or above the line that holds it, or `high` or
/// `low` of an id whose number is not one dice group with no keep. The `let:` numbers come
/// first in `Rule::exprs`, so the one at `index` reads only the ids above it.
fn find_let_read_problem(rule: &Rule) -> Option<(String, String)> {
    let lets = rule.lets.as_slice();
    for (index, expr) in rule.exprs().into_iter().enumerate() {
        let readable = lets.get(..index).unwrap_or(lets);
        for read in expr.reads() {
            let Expr::Let { id, keep } = read else {
                continue;
            };
            let (problem, hint) = match (readable.iter().find(|(held, _)| held == id), keep) {
                (Some((_, Expr::Dice { keep: None, .. })), _) | (Some(_), None) => continue,
                (Some(_), Some(_)) => (
                    format!("reads high or low of '{id}', which is not one dice group with no keep"),
                    "read high or low only of an id whose number is dice such as 2d8, with no keep such as kh1",
                ),
                (None, _) if lets.iter().any(|(held, _)| held == id) => (
                    format!("reads '{id}' in let: at or above the line that holds it"),
                    "a let: number reads only the ids written above it",
                ),
                (None, _) => (
                    format!("reads '{id}', which its let: does not hold"),
                    "add the id to let:, or read an id that is there",
                ),
            };
            return Some((problem, hint.to_string()));
        }
    }
    None
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
        Some(actor) => grammar::Subject::Id(actor.to_string()),
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
    Err("a change needs arrive, leave, meter, flag, or item".to_string())
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
            subject: actor_of(&item.subject)?,
            id: item.id.clone(),
            count: (item.count.as_ref()).map_or(Ok(Expr::Num(1)), refuse_dice)?,
        },
        CondYaml::Meter(meter) => Cond::Meter {
            of: object_of(&meter.of)?,
            key: meter.id.clone(),
            op: op_of(&meter.op)?,
            value: refuse_dice(&meter.value)?,
        },
        CondYaml::Flag(flag) => Cond::Flag {
            of: object_of(&flag.of)?,
            key: flag.key.clone(),
            is: flag.is_,
        },
        CondYaml::At(at) => Cond::At {
            subject: actor_of(&at.subject)?,
            place: at.place.clone(),
        },
        CondYaml::Cycle(cycle) => {
            let from = refuse_dice(&cycle.from)?;
            let to = refuse_dice(&cycle.to)?;
            if from.constant().is_some() && from.constant() == to.constant() {
                return Err("cycle needs from and to that differ".into());
            }
            Cond::Cycle {
                id: cycle.id.clone(),
                from,
                to,
            }
        }
        CondYaml::Players(players) => Cond::Players {
            op: op_of(&players.op)?,
            value: refuse_dice(&players.value)?,
        },
        CondYaml::NoteExists(id) => Cond::NoteExists(id.clone()),
        CondYaml::Wielding(wielding) => Cond::Wielding {
            subject: actor_of(&wielding.subject)?,
            id: wielding.id.clone(),
        },
        CondYaml::All(all) => Cond::All(all.iter().map(cond_of).collect::<Result<_, _>>()?),
        CondYaml::Any(any) => Cond::Any(any.iter().map(cond_of).collect::<Result<_, _>>()?),
        CondYaml::Not(not) => Cond::Not(Box::new(cond_of(not)?)),
    })
}

pub(super) fn cost_of(yaml: &CostYaml) -> Result<Cost, Fault> {
    Ok(Cost {
        ticks: (yaml.ticks.as_ref()).map_or(Ok(Expr::Num(0)), refuse_dice)?,
        meters: counts_of(&yaml.meters)?,
        items: counts_of(&yaml.items)?,
    })
}

fn counts_of(yaml: &Counts) -> Result<Vec<(String, Expr)>, Fault> {
    (yaml.iter())
        .map(|(key, n)| Ok((key.clone(), refuse_dice(n)?)))
        .collect()
}

fn check_of(
    total: &Spanned<ExprYaml>,
    success_at: &Spanned<ExprYaml>,
    partial_at: Option<&Spanned<ExprYaml>>,
) -> Result<rule::Check, Fault> {
    Ok(rule::Check {
        total: expr_of(total)?,
        success_at: expr_of(success_at)?,
        partial_at: partial_at.map(expr_of).transpose()?,
    })
}

/// A number in a condition or a cost, refused on its line when it rolls dice.
fn refuse_dice(yaml: &Spanned<ExprYaml>) -> Result<Expr, Fault> {
    let expr = expr_of(yaml)?;
    if expr.needs_dice() {
        return Err(Fault {
            message: "a condition or a cost cannot roll dice".into(),
            spot: spot_of(yaml),
        });
    }
    Ok(expr)
}

/// One `let:` id and its number. An id no number could read back is refused on its line.
fn let_of(id: &str, number: &Spanned<ExprYaml>) -> Result<(String, Expr), Fault> {
    if !expr::is_let_id(id) {
        return Err(Fault {
            message: format!(
                "let: id '{id}' is not a word a number can read; use letters, digits and _, \
                 not starting with a digit, and not an owner word or min, max, high or low"
            ),
            spot: spot_of(number),
        });
    }
    Ok((id.to_string(), expr_of(number)?))
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
        },
        EffectYaml::Flag(flag) if flag.key == "busy" => {
            return Err("flag key busy is not a flag".into());
        }
        EffectYaml::Flag(flag) => EffectTemplate::Flag {
            of: object_of(&flag.of)?,
            key: flag.key.clone(),
            value: flag.value,
        },
        EffectYaml::Modifier(modifier) => {
            let ticks = expr_of(&modifier.ticks)?;
            if ticks.constant().is_some_and(|ticks| ticks < 1) {
                return Err(
                    "a modifier effect has ticks below 1, so set ticks to 1 or more".into(),
                );
            }
            EffectTemplate::Modifier {
                to: actor_of(&modifier.to)?,
                id: modifier.id.clone(),
                add: expr_of(&modifier.add)?,
                ticks,
            }
        }
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
        },
        EffectYaml::Memory(memory) => EffectTemplate::Memory {
            to: actor_of(&memory.to)?,
            text: memory.text.clone(),
        },
        EffectYaml::Item(item) => EffectTemplate::Item {
            of: object_of(&item.of)?,
            id: item.id.clone(),
            add: expr_of(&item.add)?,
            max: item.max.as_ref().map(expr_of).transpose()?,
        },
        EffectYaml::Wield(wield) => EffectTemplate::Wield {
            subject: actor_of(&wield.subject)?,
            id: wield.id.clone(),
        },
        EffectYaml::Unwield(unwield) => EffectTemplate::Unwield {
            subject: actor_of(&unwield.subject)?,
        },
        EffectYaml::Check(nested) => {
            let list_of =
                |list: &[EffectYaml]| list.iter().map(effect_of).collect::<Result<Vec<_>, _>>();
            EffectTemplate::Check {
                by: match &nested.by {
                    Some(by) => actor_of(by)?,
                    None => grammar::Actor::Self_,
                },
                check: check_of(
                    &nested.total,
                    &nested.success_at,
                    nested.partial_at.as_ref(),
                )?,
                success: list_of(&nested.success)?,
                partial: list_of(&nested.partial)?,
                miss: list_of(&nested.miss)?,
            }
        }
    })
}

/// What is wrong with `check`'s partial band, given `partial`, the list its partial outcome
/// runs, as the words after the rule's id and the hint: a `partial_at` with no list, a list
/// with no `partial_at`, or a constant `partial_at` at or above a constant `success_at`.
fn find_partial_band_problem(
    check: &rule::Check,
    partial: &[EffectTemplate],
) -> Option<(&'static str, &'static str)> {
    let band = check.partial_at.as_ref();
    if band.is_some() && partial.is_empty() {
        return Some((
            "sets partial_at without a partial list",
            "add a partial list or remove partial_at",
        ));
    }
    if band.is_none() && !partial.is_empty() {
        return Some((
            "lists partial without partial_at",
            "set partial_at under check",
        ));
    }
    let constants = band
        .and_then(Expr::constant)
        .zip(check.success_at.constant());
    constants
        .is_some_and(|(partial_at, success_at)| partial_at >= success_at)
        .then_some((
            "sets partial_at at or above success_at",
            "set partial_at strictly below success_at",
        ))
}

fn object_of(yaml: &Spanned<String>) -> Result<grammar::Object, Fault> {
    grammar::Object::parse(&yaml.value).map_err(|message| Fault {
        message,
        spot: spot_of(yaml),
    })
}

/// The actor a slot that needs one names, refused on the word's line when it is another owner.
fn actor_of(yaml: &Spanned<String>) -> Result<grammar::Actor, Fault> {
    grammar::Actor::parse(&yaml.value).map_err(|message| Fault {
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
    let doc: VerbsDoc = load_tagged(rel, text, VERBS_SCHEMA)?;
    compile(rel, doc.rules, &Owner::Verb)
}
