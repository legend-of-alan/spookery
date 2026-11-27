//! The compile step that turns a rule list's typed YAML into rules.

use serde_saphyr::Spanned;
use spookery_protocol::{is_built_in, Seat};

use crate::canon::lint;
use crate::canon::refusal::{file_location_of, CompileError, Diagnostics, FileText, Locator};
use crate::diagnostic::{schema_error, Diagnostic, FileLocation};
use crate::rules::cond::{self, Cond};
use crate::rules::cost::Cost;
use crate::rules::expr::{self, stamp_file_locations, Expr};
use crate::rules::rule::{
    self, ChangeWatch, EffectTemplate, MoveTo, NarrateTo, Rule, Scope, TargetKind,
};
use crate::rules::{grammar, roll};

use super::compile;
use super::parse::{
    ChangeYaml, CondYaml, CostYaml, CountsYaml, EffectYaml, ExprYaml, IfYaml, RuleYaml, WhenYaml,
};

pub(in crate::canon) fn rules(
    file: &str,
    entries: Vec<RuleYaml>,
    scope: &Scope,
) -> Result<Vec<Rule>, Vec<Diagnostic>> {
    let file_text = FileText::without_text(file);
    let mut diagnostics = Diagnostics::default();
    let mut rules: Vec<Rule> = Vec::new();
    let mut verb_ids = Vec::new();
    for (index, entry) in entries.into_iter().enumerate() {
        let Some(rule) = diagnostics.keep(compile::rule(file, index, entry, scope)) else {
            continue;
        };
        if let rule::Kind::Verb { verb_id, .. } = &rule.kind {
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

fn rule(file: &str, index: usize, entry: RuleYaml, scope: &Scope) -> Result<Rule, Vec<Diagnostic>> {
    let file_text = FileText::without_text(file);
    let mut diagnostics = Diagnostics::default();
    let local_id = entry.id.value.clone();
    if local_id.is_empty() || local_id == "say" {
        diagnostics.push(file_text.refuse_at_rule_id(
            index,
            &entry.id,
            "a rule id is empty or is say".into(),
            "set id to a short rule id; say is speech",
        ));
    }
    let kind = match compile::kind(&entry.when.value, scope) {
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
    let if_ = match entry.if_.as_ref().map(compile::conds).transpose() {
        Ok(list) => list.unwrap_or_default(),
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            Vec::new()
        }
    };
    let start_cost = match entry.cost.as_ref().map(compile::cost).transpose() {
        Ok(cost) => cost,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            None
        }
    };
    let end_cost_yaml = entry.end.as_ref().and_then(|end| end.cost.as_ref());
    let end_cost = match end_cost_yaml.map(compile::cost).transpose() {
        Ok(Some(_)) if end_cost_yaml.is_some_and(|cost| cost.ticks.is_some()) => {
            diagnostics.push(schema_error(
                file,
                &format!("rule '{local_id}' has ticks in its end cost"),
                "put the action's ticks in cost; the end takes no time",
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
        .map(|(id, number)| compile::let_(id, number))
        .collect::<Result<Vec<_>, CompileError>>();
    let lets = match lets {
        Ok(lets) => lets,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            Vec::new()
        }
    };
    let check = match (entry.check.as_ref())
        .map(|yaml| compile::check(&yaml.total, &yaml.success_at, yaml.partial_at.as_ref()))
        .transpose()
    {
        Ok(check) => check,
        Err(error) => {
            diagnostics.push(file_text.refuse_error("", error));
            None
        }
    };
    let do_ = compile::effect_templates(file, &entry.do_)?;
    let success = compile::effect_templates(file, &entry.success)?;
    let partial = compile::effect_templates(file, &entry.partial)?;
    let miss = compile::effect_templates(file, &entry.miss)?;
    if let Some(kind) = &kind {
        let has_action_fields = start_cost.is_some()
            || entry.start.is_some()
            || entry.end.is_some()
            || entry.cooldown_ticks > 0
            || entry.cancellable;
        diagnostics.extend(lint::rule::find_change_with_action_fields(
            &file_text,
            index,
            &entry.id,
            kind,
            has_action_fields,
        ));
        let outcome_lists = [&success[..], &partial, &miss];
        diagnostics.extend(lint::rule::find_outcome_without_check(
            &file_text,
            index,
            &entry.id,
            check.as_ref(),
            outcome_lists,
        ));
        diagnostics.extend(lint::rule::find_check_without_success(
            &file_text,
            index,
            &entry.id,
            check.as_ref(),
            outcome_lists,
        ));
        let own_check = check.as_ref().map(|check| (check, &partial[..]));
        let effect_lists = [&do_[..], &success, &partial, &miss];
        diagnostics.extend(lint::rule::find_broken_partial_band(
            &file_text,
            index,
            &entry.id,
            own_check,
            effect_lists,
        ));
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
        start_cost,
        end_cost,
        start_counts_as_act: entry
            .start
            .as_ref()
            .and_then(|start| start.counts_as_act)
            .unwrap_or(true),
        end_counts_as_act: entry.end.as_ref().is_some_and(|end| end.counts_as_act),
        cooldown_ticks: entry.cooldown_ticks,
        cancellable: entry.cancellable,
        do_,
        success,
        partial,
        miss,
        file: file.to_string(),
        index,
    };
    let mut refusals = lint::rule::find_misplaced_target(&rule, &entry.id);
    if refusals.is_empty() {
        refusals = lint::rule::find_failing_let_read(&rule, &entry.id);
    }
    if refusals.is_empty() {
        Ok(rule)
    } else {
        Err(refusals)
    }
}

fn kind(yaml: &WhenYaml, scope: &Scope) -> Result<rule::Kind, String> {
    match yaml {
        WhenYaml::Word(word) if word == "habit" => Ok(rule::Kind::Habit),
        WhenYaml::Word(word) => Err(format!("when '{word}' is not habit")),
        WhenYaml::Map(map) => match (&map.verb_id, &map.change) {
            (Some(verb_id), None) => {
                if verb_id.is_empty() {
                    return Err("verb is empty".to_string());
                }
                if is_built_in(Seat::Player, verb_id) {
                    return Err(format!(
                        "verb '{verb_id}' is a built-in command; give the rule another verb"
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
                Ok(rule::Kind::Verb {
                    verb_id: verb_id.clone(),
                    target,
                })
            }
            (None, Some(change)) => compile::change_watch(change, scope).map(rule::Kind::Change),
            _ => Err("when needs one of verb or change".to_string()),
        },
    }
}

/// The watch in `yaml`, with `here` on a place rule read as that place's id.
fn change_watch(yaml: &ChangeYaml, scope: &Scope) -> Result<ChangeWatch, String> {
    let here_id = match scope {
        Scope::Place(id) => Some(id.as_str()),
        _ => None,
    };
    let subject = match yaml.subject.as_deref() {
        None | Some("any") => grammar::Subject::Any,
        Some("pc") => grammar::Subject::Pc,
        Some("npc") => grammar::Subject::Npc,
        Some(actor_id) => grammar::Subject::Id(actor_id.to_string()),
    };
    if let Some(place_id) = &yaml.arrive_place_id {
        let place_id = here_id.filter(|_| place_id == "here").unwrap_or(place_id);
        return Ok(ChangeWatch::Arrive {
            place_id: place_id.to_string(),
            subject,
        });
    }
    if let Some(place_id) = &yaml.leave_place_id {
        let place_id = here_id.filter(|_| place_id == "here").unwrap_or(place_id);
        return Ok(ChangeWatch::Leave {
            place_id: place_id.to_string(),
            subject,
        });
    }
    if let Some(id) = &yaml.meter_id {
        let crosses = yaml
            .crosses
            .as_deref()
            .ok_or_else(|| "a meter change needs crosses".to_string())?;
        let (op, bound) = compile::crosses(crosses)?;
        return Ok(ChangeWatch::Meter {
            id: id.clone(),
            op,
            bound,
        });
    }
    if let Some(id) = &yaml.flag_id {
        let becomes = yaml
            .becomes
            .ok_or_else(|| "a flag change needs becomes".to_string())?;
        return Ok(ChangeWatch::Flag {
            id: id.clone(),
            becomes,
        });
    }
    if let Some(id) = &yaml.item_id {
        let gained = match (yaml.gained, yaml.lost) {
            (Some(true), _) => true,
            (_, Some(true)) => false,
            _ => return Err("an item change needs gained or lost".to_string()),
        };
        return Ok(ChangeWatch::Item {
            id: id.clone(),
            gained,
        });
    }
    Err("a change needs arrive, leave, meter, flag, or item".to_string())
}

fn conds(yaml: &IfYaml) -> Result<Vec<Cond>, CompileError> {
    match yaml {
        IfYaml::List(list) => list.iter().map(compile::cond).collect(),
        IfYaml::One(one) => Ok(vec![compile::cond(one)?]),
    }
}

pub(in crate::canon) fn cond(yaml: &CondYaml) -> Result<Cond, CompileError> {
    Ok(match yaml {
        CondYaml::HasItem(item) => Cond::HasItem {
            subject: compile::actor(&item.subject)?,
            id: item.id.clone(),
            count: (item.count.as_ref()).map_or(Ok(Expr::Num(1)), compile::expr_without_dice)?,
        },
        CondYaml::Meter(meter) => Cond::Meter {
            of: compile::object(&meter.of)?,
            id: meter.id.clone(),
            op: compile::op(&meter.op)?,
            value: compile::expr_without_dice(&meter.value)?,
        },
        CondYaml::Flag(flag) => Cond::Flag {
            of: compile::object(&flag.of)?,
            id: flag.id.clone(),
            is: flag.is_,
        },
        CondYaml::At(at) => Cond::At {
            subject: compile::actor(&at.subject)?,
            place_id: at.place_id.clone(),
        },
        CondYaml::Cycle(cycle) => {
            let from = compile::expr_without_dice(&cycle.from)?;
            let to = compile::expr_without_dice(&cycle.to)?;
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
            op: compile::op(&players.op)?,
            value: compile::expr_without_dice(&players.value)?,
        },
        CondYaml::NoteExists(id) => Cond::NoteExists(id.clone()),
        CondYaml::Wielding(wielding) => Cond::Wielding {
            subject: compile::actor(&wielding.subject)?,
            id: wielding.id.clone(),
        },
        CondYaml::All(all) => Cond::All(all.iter().map(compile::cond).collect::<Result<_, _>>()?),
        CondYaml::Any(any) => Cond::Any(any.iter().map(compile::cond).collect::<Result<_, _>>()?),
        CondYaml::Not(not) => Cond::Not(Box::new(compile::cond(not)?)),
    })
}

pub(in crate::canon) fn cost(yaml: &CostYaml) -> Result<Cost, CompileError> {
    Ok(Cost {
        ticks: (yaml.ticks.as_ref()).map_or(Ok(Expr::Num(0)), compile::expr_without_dice)?,
        meters: compile::counts(&yaml.meters)?,
        items: compile::counts(&yaml.items)?,
    })
}

fn counts(yaml: &CountsYaml) -> Result<Vec<(String, Expr)>, CompileError> {
    (yaml.iter())
        .map(|(key, n)| Ok((key.clone(), compile::expr_without_dice(n)?)))
        .collect()
}

fn check(
    total: &Spanned<ExprYaml>,
    success_at: &Spanned<ExprYaml>,
    partial_at: Option<&Spanned<ExprYaml>>,
) -> Result<roll::Check, CompileError> {
    Ok(roll::Check {
        total: compile::expr(total)?,
        success_at: compile::expr(success_at)?,
        partial_at: partial_at.map(compile::expr).transpose()?,
    })
}

/// A number in a cond or a cost, refused on its line when it rolls dice.
fn expr_without_dice(yaml: &Spanned<ExprYaml>) -> Result<Expr, CompileError> {
    let expr = compile::expr(yaml)?;
    if expr.needs_dice() {
        return Err(CompileError {
            message: "a condition or a cost cannot roll dice".into(),
            file_location: file_location_of(yaml),
        });
    }
    Ok(expr)
}

/// One `let:` id and its number. An id no number could read back is refused on its line.
fn let_(id: &str, number: &Spanned<ExprYaml>) -> Result<(String, Expr), CompileError> {
    if !expr::is_let_id(id) {
        return Err(CompileError {
            message: format!(
                "let: id '{id}' is not a word a number can read; use letters, digits and _, \
                 not starting with a digit, and not an owner word or min, max, high or low"
            ),
            file_location: file_location_of(number),
        });
    }
    Ok((id.to_string(), compile::expr(number)?))
}

fn expr(yaml: &Spanned<ExprYaml>) -> Result<Expr, CompileError> {
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

fn effect_templates(
    file: &str,
    list: &[EffectYaml],
) -> Result<Vec<EffectTemplate>, Vec<Diagnostic>> {
    let mut out = Vec::new();
    for effect in list {
        match compile::effect_template(effect) {
            Ok(effect) => out.push(effect),
            Err(error) => {
                return Err(vec![FileText::without_text(file).refuse_error("", error)]);
            }
        }
    }
    Ok(out)
}

fn effect_template(yaml: &EffectYaml) -> Result<EffectTemplate, CompileError> {
    Ok(match yaml {
        EffectYaml::Meter(meter) if meter.add.is_some() == meter.set.is_some() => {
            return Err("a meter effect needs add or set, not both".into());
        }
        EffectYaml::Meter(meter) => EffectTemplate::Meter {
            of: compile::object(&meter.of)?,
            id: meter.id.clone(),
            add: meter.add.as_ref().map(compile::expr).transpose()?,
            set: meter.set.as_ref().map(compile::expr).transpose()?,
        },
        EffectYaml::Flag(flag) if flag.id == "busy" => {
            return Err("flag id busy is not a flag".into());
        }
        EffectYaml::Flag(flag) => EffectTemplate::Flag {
            of: compile::object(&flag.of)?,
            id: flag.id.clone(),
            value: flag.value,
        },
        EffectYaml::Modifier(modifier) => {
            let ticks = compile::expr(&modifier.ticks)?;
            if ticks.constant().is_some_and(|ticks| ticks < 1) {
                return Err(
                    "a modifier effect has ticks below 1, so set ticks to 1 or more".into(),
                );
            }
            EffectTemplate::Modifier {
                to: compile::actor(&modifier.to)?,
                id: modifier.id.clone(),
                add: compile::expr(&modifier.add)?,
                ticks,
            }
        }
        EffectYaml::Location(location) => EffectTemplate::Move {
            to: match (&location.to, &location.via) {
                (Some(word), None) if word == "target" => MoveTo::Target,
                (Some(place), None) => MoveTo::Place(place.clone()),
                (None, Some(via)) => MoveTo::Via(via.clone()),
                _ => return Err("location needs to or via".into()),
            },
            teleport: location.teleport,
        },
        EffectYaml::Note(note)
            if note.id.is_empty() || note.text.is_empty() || note.to_ids.is_empty() =>
        {
            return Err("a note needs id, text, and to".into());
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
            to: compile::actor(&memory.to)?,
            text: memory.text.clone(),
        },
        EffectYaml::Item(item) => EffectTemplate::Item {
            of: compile::object(&item.of)?,
            id: item.id.clone(),
            add: compile::expr(&item.add)?,
            max: item.max.as_ref().map(compile::expr).transpose()?,
        },
        EffectYaml::Wield(wield) => EffectTemplate::Wield {
            subject: compile::actor(&wield.subject)?,
            id: wield.id.clone(),
        },
        EffectYaml::Unwield(unwield) => EffectTemplate::Unwield {
            subject: compile::actor(&unwield.subject)?,
        },
        EffectYaml::Check(nested) => {
            let list_of = |list: &[EffectYaml]| {
                list.iter()
                    .map(compile::effect_template)
                    .collect::<Result<Vec<_>, _>>()
            };
            EffectTemplate::Check {
                by: match &nested.by {
                    Some(by) => compile::actor(by)?,
                    None => grammar::Actor::Self_,
                },
                check: compile::check(
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

fn object(yaml: &Spanned<String>) -> Result<grammar::Object, CompileError> {
    grammar::Object::parse(&yaml.value).map_err(|message| CompileError {
        message,
        file_location: file_location_of(yaml),
    })
}

/// The actor a slot that needs one names, refused on the word's line when it is another entity.
fn actor(yaml: &Spanned<String>) -> Result<grammar::Actor, CompileError> {
    grammar::Actor::parse(&yaml.value).map_err(|message| CompileError {
        message,
        file_location: file_location_of(yaml),
    })
}

fn op(word: &str) -> Result<cond::Op, String> {
    cond::Op::parse(word).ok_or_else(|| format!("op '{word}' is not <, <=, ==, >=, or >"))
}

fn crosses(text: &str) -> Result<(cond::Op, i64), String> {
    let text = text.trim();
    let split = text.find(|ch| !matches!(ch, '<' | '=' | '>'));
    let (op, bound) = text.split_at(split.unwrap_or(text.len()));
    cond::Op::parse(op)
        .zip(bound.trim().parse::<i64>().ok())
        .ok_or_else(|| format!("crosses '{text}' needs an operator and a number"))
}
