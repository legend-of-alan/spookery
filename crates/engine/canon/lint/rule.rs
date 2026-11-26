//! Lints that compile runs on one rule: a misplaced target, a `let:` read that fails, and a
//! broken partial band.

use serde_saphyr::Spanned;

use crate::canon::refusal::FileText;
use crate::diagnostic::Diagnostic;
use crate::rules::expr::Expr;
use crate::rules::roll;
use crate::rules::rule::{EffectTemplate, Rule};

/// The first key in `rule` whose `target` needs a target kind other than the rule's own,
/// refused on the line of `rule_id`.
pub(in crate::canon) fn find_misplaced_target(
    rule: &Rule,
    rule_id: &Spanned<String>,
) -> Vec<Diagnostic> {
    let target_kind = rule.kind.target_kind();
    let Some((key, needs)) = rule.mentions().find_misplaced_target(target_kind) else {
        return Vec::new();
    };
    let (needs, rule_target) = (needs.as_str(), target_kind.as_str());
    let message = format!("says target in {key}, but its target is {rule_target}");
    let hint = format!("give the verb target: {needs}, or name the {needs} by its id");
    let file_text = FileText::without_text(&rule.file);
    refuse(&file_text, rule.index, rule_id, &message, &hint)
}

/// The first `let:` read in `rule` that fails, refused on the line of `rule_id`: an id its
/// `let:` does not hold, an id a `let:` number reads at or above the line that holds it, or
/// `high` or `low` of an id whose number is not one dice group with no keep. The `let:`
/// numbers come first in `Rule::exprs`, so the one at `expr_index` reads only the ids above it.
pub(in crate::canon) fn find_failing_let_read(
    rule: &Rule,
    rule_id: &Spanned<String>,
) -> Vec<Diagnostic> {
    let file_text = FileText::without_text(&rule.file);
    let lets = rule.lets.as_slice();
    for (expr_index, expr) in rule.exprs().into_iter().enumerate() {
        let readable = lets.get(..expr_index).unwrap_or(lets);
        for read in expr.reads() {
            let Expr::Let { id, keep } = read else {
                continue;
            };
            let (message, hint) = match (readable.iter().find(|(bound, _)| bound == id), keep) {
                (Some((_, Expr::Dice { keep: None, .. })), _) | (Some(_), None) => continue,
                (Some(_), Some(_)) => (
                    format!("reads high or low of '{id}', which is not one dice group with no keep"),
                    "read high or low only of an id whose number is dice such as 2d8, with no keep such as kh1",
                ),
                (None, _) if lets.iter().any(|(bound, _)| bound == id) => (
                    format!("reads '{id}' in let: at or above the line that holds it"),
                    "a let: number reads only the ids written above it",
                ),
                (None, _) => (
                    format!("reads '{id}', which its let: does not hold"),
                    "add the id to let:, or read an id that is there",
                ),
            };
            return refuse(&file_text, rule.index, rule_id, &message, hint);
        }
    }
    Vec::new()
}

/// The first broken partial band in rule `rule_index`, refused on the line of `rule_id`: in
/// `own_check`, the rule's own check with its partial list, or in a check nested in one of
/// `effect_lists`.
pub(in crate::canon) fn find_broken_partial_band(
    file_text: &FileText,
    rule_index: usize,
    rule_id: &Spanned<String>,
    own_check: Option<(&roll::Check, &[EffectTemplate])>,
    effect_lists: [&[EffectTemplate]; 4],
) -> Vec<Diagnostic> {
    let mut checks: Vec<_> = own_check.into_iter().collect();
    for effect in effect_lists.into_iter().flatten() {
        effect.visit(|effect| {
            if let EffectTemplate::Check { check, partial, .. } = effect {
                checks.push((check, &partial[..]));
            }
        });
    }
    let band_message =
        (checks.into_iter()).find_map(|(check, partial)| partial_band_message(check, partial));
    let Some((message, hint)) = band_message else {
        return Vec::new();
    };
    refuse(file_text, rule_index, rule_id, message, hint)
}

/// The refusal of rule `rule_index`, on the line of `rule_id`, with the words after its id.
fn refuse(
    file_text: &FileText,
    rule_index: usize,
    rule_id: &Spanned<String>,
    message: &str,
    hint: &str,
) -> Vec<Diagnostic> {
    let message = format!("rule '{}' {message}", rule_id.value);
    vec![file_text.refuse_at_rule_id(rule_index, rule_id, message, hint)]
}

/// What is wrong with `check`'s partial band, given `partial`, the list its partial outcome
/// runs, as the words after the rule's id and the hint: a `partial_at` with no list, a list
/// with no `partial_at`, or a constant `partial_at` at or above a constant `success_at`.
fn partial_band_message(
    check: &roll::Check,
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
