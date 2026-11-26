//! A rule list: its typed YAML and the compile step that turns it into rules.

use serde_saphyr::Spanned;
use spookery_protocol::{is_built_in, Seat};

use crate::canon::refusal::{file_location_of, CompileError, Diagnostics, FileText, Locator};
use crate::diagnostic::{schema_error, Diagnostic};
use crate::rules::expr::{self, stamp_file_locations, Expr};
use crate::rules::grammar;
use crate::rules::rule::{
    self, ChangeWatch, Cond, Cost, EffectTemplate, FileLocation, LocationTo, NarrateTo, Op, Phase,
    Rule, RuleKind, Scope, TargetKind,
};

use parse::{
    ChangeYaml, CondYaml, CostYaml, CountsYaml, EffectYaml, ExprYaml, IfYaml, RuleYaml, WhenYaml,
};

pub(super) mod parse;

pub(super) fn compile(
    file: &str,
    entries: Vec<RuleYaml>,
    scope: &Scope,
) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let file_text = FileText::without_text(file);
    let mut diagnostics = Diagnostics::default();
    let mut rules: Vec<Rule> = Vec::new();
    let mut verb_ids = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(rule) = diagnostics.keep(compile_rule(file, index, entry, scope)) else {
            continue;
        };
        if let RuleKind::Verb { verb_id, .. } = &rule.kind {
            let hint = "keep one rule for that verb in this file";
            let duplicate_words = ("two verb rules for", "at the same level", hint);
            let is_duplicate = verb_ids.contains(verb_id);
            diagnostics.extend(is_duplicate.then(|| {
                file_text.refuse_duplicate(verb_id, "", Locator::Nowhere, duplicate_words)
            }));
            verb_ids.push(verb_id.clone());
        }
        let duplicate_words = ("rule", "is listed twice", "keep one rule with this id");
        let is_duplicate = rules.iter().any(|earlier| earlier.id == rule.id);
        diagnostics.extend(is_duplicate.then(|| {
            file_text.refuse_duplicate(&rule.local_id, "", Locator::Nowhere, duplicate_words)
        }));
        rules.push(rule);
    }
    diagnostics.into_result(rules)
}

fn compile_rule(
    file: &str,
    index: usize,
    entry: RuleYaml,
    scope: &Scope,
) -> Result<Rule, Vec<Diagnostic>> {
    let file_text = FileText::without_text(file);
    let mut diagnostics = Diagnostics::default();
    let local_id = entry.id.value.clone();
    if local_id.is_empty() || local_id == "say" {
        diagnostics.push(file_text.refuse_at(
            &format!("rules[{index}].id"),
            Locator::FileLocation(file_location_of(&entry.id)),
            "a rule id is empty or is say".into(),
            "set id to a short rule id; say is speech",
        ));
    }
    let kind = match compile_rule_kind(&entry.when.value) {
        Ok(kind) => Some(kind),
        Err(message) => {
            diagnostics.push(file_text.refuse_at(
                &format!("rules[{index}].when"),
                Locator::FileLocation(file_location_of(&entry.when)),
                message,
                "set when to habit, a verb, or a change",
            ));
            None
        }
    };
    let if_ = match entry.if_.as_ref().map(compile_conds).transpose() {
        Ok(list) => list.unwrap_or_default(),
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            Vec::new()
        }
    };
    let cost = match entry.cost.as_ref().map(compile_cost).transpose() {
        Ok(cost) => cost,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            None
        }
    };
    let finish_cost_yaml = entry
        .finish
        .as_ref()
        .and_then(|finish| finish.cost.as_ref());
    let finish_cost = match finish_cost_yaml.map(compile_cost).transpose() {
        Ok(Some(_)) if finish_cost_yaml.is_some_and(|cost| cost.ticks.is_some()) => {
            diagnostics.push(schema_error(
                file,
                &format!("rule '{local_id}' has ticks in its finish cost"),
                "put the action's ticks in cost; the finish takes no time",
            ));
            None
        }
        Ok(cost) => cost,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            None
        }
    };
    let mut lets: Vec<_> = entry.lets.iter().collect();
    lets.sort_by_key(|(_, number)| (number.referenced.line(), number.referenced.column()));
    let lets = (lets.into_iter())
        .map(|(id, number)| compile_let(id, number))
        .collect::<Result<Vec<_>, CompileError>>();
    let lets = match lets {
        Ok(lets) => lets,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            Vec::new()
        }
    };
    let check = match (entry.check.as_ref())
        .map(|yaml| compile_check(&yaml.total, &yaml.success_at, yaml.partial_at.as_ref()))
        .transpose()
    {
        Ok(check) => check,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            None
        }
    };
    let do_ = compile_effects(file, &entry.do_)?;
    let success = compile_effects(file, &entry.success)?;
    let partial = compile_effects(file, &entry.partial)?;
    let miss = compile_effects(file, &entry.miss)?;
    if let Some(kind) = &kind {
        let timed = cost.is_some()
            || entry.start.is_some()
            || entry.finish.is_some()
            || entry.cooldown > 0
            || entry.cancellable;
        if matches!(kind, RuleKind::Change(_)) && timed {
            diagnostics.push(schema_error(
                file,
                "a change rule has a cost, start, finish, cooldown, or cancellable",
                "remove them; a change is a consequence, not an action",
            ));
        }
        if check.is_none() && (!success.is_empty() || !partial.is_empty() || !miss.is_empty()) {
            diagnostics.push(schema_error(
                file,
                &format!("rule '{local_id}' lists an outcome without a check"),
                "add check or move those effects to do",
            ));
        }
        if check.is_some() && success.is_empty() && (!partial.is_empty() || !miss.is_empty()) {
            diagnostics.push(schema_error(
                file,
                &format!("rule '{local_id}' has a check without a success list"),
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
        let band_message = (checks.into_iter())
            .find_map(|(check, partial)| find_partial_band_message(check, partial));
        if let Some((message, hint)) = band_message {
            diagnostics.push(file_text.refuse_at(
                &format!("rules[{index}].id"),
                Locator::FileLocation(file_location_of(&entry.id)),
                format!("rule '{local_id}' {message}"),
                hint,
            ));
        }
    }
    let kind = diagnostics.into_result(kind)?.expect("kind checked");
    let rule = Rule {
        id: scope.rule_id(&local_id),
        local_id,
        scope: scope.clone(),
        kind,
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
    let target_kind = rule.kind.target_kind();
    let misplaced_target = rule.mentions().find_misplaced_target(target_kind);
    let misplaced_message = misplaced_target.map(|(key, needs)| {
        let (needs, target) = (needs.as_str(), target_kind.as_str());
        (
            format!("says target in {key}, but its target is {target}"),
            format!("give the verb target: {needs}, or name the {needs} by its id"),
        )
    });
    if let Some((message, hint)) = misplaced_message.or_else(|| find_let_read_message(&rule)) {
        return Err(vec![file_text.refuse_at(
            &format!("rules[{index}].id"),
            Locator::FileLocation(file_location_of(&entry.id)),
            format!("rule '{}' {message}", rule.local_id),
            &hint,
        )]);
    }
    Ok(rule)
}

/// The first `let:` read in `rule` that fails, as the message and the hint: an id its `let:`
/// does not hold, an id a `let:` number reads at or above the line that holds it, or `high` or
/// `low` of an id whose number is not one dice group with no keep. The `let:` numbers come
/// first in `Rule::exprs`, so the one at `index` reads only the ids above it.
fn find_let_read_message(rule: &Rule) -> Option<(String, String)> {
    let lets = rule.lets.as_slice();
    for (index, expr) in rule.exprs().into_iter().enumerate() {
        let readable = lets.get(..index).unwrap_or(lets);
        for read in expr.reads() {
            let Expr::Let { id, keep } = read else {
                continue;
            };
            let (message, hint) = match (readable.iter().find(|(held, _)| held == id), keep) {
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
            return Some((message, hint.to_string()));
        }
    }
    None
}

fn compile_rule_kind(yaml: &WhenYaml) -> Result<RuleKind, String> {
    match yaml {
        WhenYaml::Word(word) if word == "habit" => Ok(RuleKind::Habit),
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
                    Some("path") => TargetKind::Path,
                    Some("item") => TargetKind::Item,
                    Some(other) => return Err(format!("target '{other}' is not a target kind")),
                };
                Ok(RuleKind::Verb {
                    verb_id: verb.clone(),
                    target,
                })
            }
            (None, Some(change)) => compile_change(change),
            _ => Err("when needs one of verb or change".to_string()),
        },
    }
}

fn compile_change(yaml: &ChangeYaml) -> Result<RuleKind, String> {
    let subject = match yaml.subject.as_deref() {
        None | Some("any") => grammar::Subject::Any,
        Some("pc") => grammar::Subject::Pc,
        Some("npc") => grammar::Subject::Npc,
        Some(actor) => grammar::Subject::Id(actor.to_string()),
    };
    if let Some(place) = &yaml.arrive {
        return Ok(RuleKind::Change(ChangeWatch::Arrive {
            place: place.clone(),
            subject,
        }));
    }
    if let Some(place) = &yaml.leave {
        return Ok(RuleKind::Change(ChangeWatch::Leave {
            place: place.clone(),
            subject,
        }));
    }
    if let Some(key) = &yaml.meter {
        let crosses = yaml
            .crosses
            .as_deref()
            .ok_or_else(|| "a meter change needs crosses".to_string())?;
        let (op, bound) = parse_crosses(crosses)?;
        return Ok(RuleKind::Change(ChangeWatch::Meter {
            key: key.clone(),
            op,
            bound,
        }));
    }
    if let Some(key) = &yaml.flag {
        let becomes = yaml
            .becomes
            .ok_or_else(|| "a flag change needs becomes".to_string())?;
        return Ok(RuleKind::Change(ChangeWatch::Flag {
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
        return Ok(RuleKind::Change(ChangeWatch::Item {
            id: id.clone(),
            gained,
        }));
    }
    Err("a change needs arrive, leave, meter, flag, or item".to_string())
}

fn compile_conds(yaml: &IfYaml) -> Result<Vec<Cond>, CompileError> {
    match yaml {
        IfYaml::List(list) => list.iter().map(compile_cond).collect(),
        IfYaml::One(one) => Ok(vec![compile_cond(one)?]),
    }
}

pub(super) fn compile_cond(yaml: &CondYaml) -> Result<Cond, CompileError> {
    Ok(match yaml {
        CondYaml::HasItem(item) => Cond::HasItem {
            subject: parse_actor(&item.subject)?,
            id: item.id.clone(),
            count: (item.count.as_ref()).map_or(Ok(Expr::Num(1)), compile_expr_without_dice)?,
        },
        CondYaml::Meter(meter) => Cond::Meter {
            of: parse_object(&meter.of)?,
            key: meter.id.clone(),
            op: parse_op(&meter.op)?,
            value: compile_expr_without_dice(&meter.value)?,
        },
        CondYaml::Flag(flag) => Cond::Flag {
            of: parse_object(&flag.of)?,
            key: flag.key.clone(),
            is: flag.is_,
        },
        CondYaml::At(at) => Cond::At {
            subject: parse_actor(&at.subject)?,
            place: at.place.clone(),
        },
        CondYaml::Cycle(cycle) => {
            let from = compile_expr_without_dice(&cycle.from)?;
            let to = compile_expr_without_dice(&cycle.to)?;
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
            op: parse_op(&players.op)?,
            value: compile_expr_without_dice(&players.value)?,
        },
        CondYaml::NoteExists(id) => Cond::NoteExists(id.clone()),
        CondYaml::Wielding(wielding) => Cond::Wielding {
            subject: parse_actor(&wielding.subject)?,
            id: wielding.id.clone(),
        },
        CondYaml::All(all) => Cond::All(all.iter().map(compile_cond).collect::<Result<_, _>>()?),
        CondYaml::Any(any) => Cond::Any(any.iter().map(compile_cond).collect::<Result<_, _>>()?),
        CondYaml::Not(not) => Cond::Not(Box::new(compile_cond(not)?)),
    })
}

pub(super) fn compile_cost(yaml: &CostYaml) -> Result<Cost, CompileError> {
    Ok(Cost {
        ticks: (yaml.ticks.as_ref()).map_or(Ok(Expr::Num(0)), compile_expr_without_dice)?,
        meters: compile_counts(&yaml.meters)?,
        items: compile_counts(&yaml.items)?,
    })
}

fn compile_counts(yaml: &CountsYaml) -> Result<Vec<(String, Expr)>, CompileError> {
    (yaml.iter())
        .map(|(key, n)| Ok((key.clone(), compile_expr_without_dice(n)?)))
        .collect()
}

fn compile_check(
    total: &Spanned<ExprYaml>,
    success_at: &Spanned<ExprYaml>,
    partial_at: Option<&Spanned<ExprYaml>>,
) -> Result<rule::Check, CompileError> {
    Ok(rule::Check {
        total: compile_expr(total)?,
        success_at: compile_expr(success_at)?,
        partial_at: partial_at.map(compile_expr).transpose()?,
    })
}

/// A number in a cond or a cost, refused on its line when it rolls dice.
fn compile_expr_without_dice(yaml: &Spanned<ExprYaml>) -> Result<Expr, CompileError> {
    let expr = compile_expr(yaml)?;
    if expr.needs_dice() {
        return Err(CompileError {
            message: "a condition or a cost cannot roll dice".into(),
            file_location: file_location_of(yaml),
        });
    }
    Ok(expr)
}

/// One `let:` id and its number. An id no number could read back is refused on its line.
fn compile_let(id: &str, number: &Spanned<ExprYaml>) -> Result<(String, Expr), CompileError> {
    if !expr::is_let_id(id) {
        return Err(CompileError {
            message: format!(
                "let: id '{id}' is not a word a number can read; use letters, digits and _, \
                 not starting with a digit, and not an owner word or min, max, high or low"
            ),
            file_location: file_location_of(number),
        });
    }
    Ok((id.to_string(), compile_expr(number)?))
}

fn compile_expr(yaml: &Spanned<ExprYaml>) -> Result<Expr, CompileError> {
    let file_location = file_location_of(yaml);
    match &yaml.value {
        ExprYaml::Literal(n) => Ok(Expr::Num(*n)),
        ExprYaml::Text(text) => {
            let mut expr = Expr::parse(text).map_err(|message| CompileError {
                message,
                file_location,
            })?;
            if let Some(FileLocation { line, column }) = file_location {
                stamp_file_locations(&mut expr, line, column);
            }
            Ok(expr)
        }
    }
}

fn compile_effects(
    file: &str,
    list: &[EffectYaml],
) -> Result<Vec<EffectTemplate>, Vec<Diagnostic>> {
    let mut out = Vec::new();
    for effect in list {
        match compile_effect(effect) {
            Ok(effect) => out.push(effect),
            Err(error) => {
                return Err(vec![FileText::without_text(file).refuse_error("", error)]);
            }
        }
    }
    Ok(out)
}

fn compile_effect(yaml: &EffectYaml) -> Result<EffectTemplate, CompileError> {
    Ok(match yaml {
        EffectYaml::Meter(meter) if meter.add.is_some() == meter.set.is_some() => {
            return Err("a meter effect needs add or set, not both".into());
        }
        EffectYaml::Meter(meter) => EffectTemplate::Meter {
            of: parse_object(&meter.of)?,
            key: meter.key.clone(),
            add: meter.add.as_ref().map(compile_expr).transpose()?,
            set: meter.set.as_ref().map(compile_expr).transpose()?,
        },
        EffectYaml::Flag(flag) if flag.key == "busy" => {
            return Err("flag key busy is not a flag".into());
        }
        EffectYaml::Flag(flag) => EffectTemplate::Flag {
            of: parse_object(&flag.of)?,
            key: flag.key.clone(),
            value: flag.value,
        },
        EffectYaml::Modifier(modifier) => {
            let ticks = compile_expr(&modifier.ticks)?;
            if ticks.constant().is_some_and(|ticks| ticks < 1) {
                return Err(
                    "a modifier effect has ticks below 1, so set ticks to 1 or more".into(),
                );
            }
            EffectTemplate::Modifier {
                to: parse_actor(&modifier.to)?,
                id: modifier.id.clone(),
                add: compile_expr(&modifier.add)?,
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
            to: parse_actor(&memory.to)?,
            text: memory.text.clone(),
        },
        EffectYaml::Item(item) => EffectTemplate::Item {
            of: parse_object(&item.of)?,
            id: item.id.clone(),
            add: compile_expr(&item.add)?,
            max: item.max.as_ref().map(compile_expr).transpose()?,
        },
        EffectYaml::Wield(wield) => EffectTemplate::Wield {
            subject: parse_actor(&wield.subject)?,
            id: wield.id.clone(),
        },
        EffectYaml::Unwield(unwield) => EffectTemplate::Unwield {
            subject: parse_actor(&unwield.subject)?,
        },
        EffectYaml::Check(nested) => {
            let list_of = |list: &[EffectYaml]| {
                list.iter()
                    .map(compile_effect)
                    .collect::<Result<Vec<_>, _>>()
            };
            EffectTemplate::Check {
                by: match &nested.by {
                    Some(by) => parse_actor(by)?,
                    None => grammar::Actor::Self_,
                },
                check: compile_check(
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
fn find_partial_band_message(
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

fn parse_object(yaml: &Spanned<String>) -> Result<grammar::Object, CompileError> {
    grammar::Object::parse(&yaml.value).map_err(|message| CompileError {
        message,
        file_location: file_location_of(yaml),
    })
}

/// The actor a slot that needs one names, refused on the word's line when it is another entity.
fn parse_actor(yaml: &Spanned<String>) -> Result<grammar::Actor, CompileError> {
    grammar::Actor::parse(&yaml.value).map_err(|message| CompileError {
        message,
        file_location: file_location_of(yaml),
    })
}

fn parse_op(word: &str) -> Result<Op, String> {
    Op::parse(word).ok_or_else(|| format!("op '{word}' is not <, <=, ==, >=, or >"))
}

fn parse_crosses(text: &str) -> Result<(Op, i64), String> {
    let text = text.trim();
    let split = text.find(|ch| !matches!(ch, '<' | '=' | '>'));
    let (op, bound) = text.split_at(split.unwrap_or(text.len()));
    Op::parse(op)
        .zip(bound.trim().parse::<i64>().ok())
        .ok_or_else(|| format!("crosses '{text}' needs an operator and a number"))
}
