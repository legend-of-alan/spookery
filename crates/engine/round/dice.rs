//! Roll keys, check rolls, and expression dice for one round.

use std::collections::BTreeMap;

use crate::ecs::id::ActorId;
use crate::hash::blake2s_256;
use crate::round::Ledger;
use crate::rules::context::Context;
use crate::rules::expr::{self, eval_with_dice};
use crate::rules::roll::Outcome;
use crate::rules::roll::{self, classify, faces, MoveReport, Roll};
use crate::rules::rule;
use crate::Reason;

pub(crate) fn shuffle_key(seed: &str, round_index: u64, actor_id: &ActorId) -> [u8; 32] {
    blake2s_256(format!("{seed}|{round_index}|{actor_id}").as_bytes())
}

/// Every roll key of one round.
pub(crate) struct Dice {
    seed: String,
    round_index: u64,
    counts: BTreeMap<(String, Option<ActorId>), u32>,
}

impl Dice {
    pub(crate) fn new(seed: String, round_index: u64) -> Self {
        Self {
            seed,
            round_index,
            counts: BTreeMap::new(),
        }
    }

    /// The one allocator: the next key for this rule and subject.
    fn next_key(&mut self, rule: &str, subject: Option<&ActorId>) -> roll::Key {
        let index = self
            .counts
            .entry((rule.to_string(), subject.cloned()))
            .or_insert(0);
        let roll_index = *index;
        *index += 1;
        roll::Key {
            seed: self.seed.clone(),
            round_index: self.round_index,
            rule_id: rule.to_string(),
            subject_id: subject.cloned(),
            index: roll_index,
        }
    }

    pub(crate) fn roller<'a>(
        &'a mut self,
        ledger: &'a mut Ledger,
        rule: &'a str,
        subject: Option<&'a ActorId>,
    ) -> Roller<'a> {
        Roller {
            dice: self,
            ledger,
            rule,
            subject,
            key: None,
            drawn: 0,
            reports: Vec::new(),
        }
    }
}

/// Expression dice and resist rolls for one rule and subject, all keyed by the rule id.
/// `into_reports` returns the resist reports. Dropping the roller discards them.
pub(crate) struct Roller<'a> {
    dice: &'a mut Dice,
    ledger: &'a mut Ledger,
    rule: &'a str,
    subject: Option<&'a ActorId>,
    key: Option<roll::Key>,
    drawn: usize,
    reports: Vec<MoveReport>,
}

impl Roller<'_> {
    /// Evaluate `check`'s `total`, then `success_at`, then `partial_at`, each writing its own
    /// dice line, and classify the total.
    pub(crate) fn roll_check(
        &mut self,
        context: &Context,
        check: &rule::Check,
    ) -> Result<Roll, Reason> {
        let total = eval_with_dice(context, &check.total, self)?;
        let success_at = eval_with_dice(context, &check.success_at, self)?;
        let partial_at = (check.partial_at.as_ref())
            .map(|expr| eval_with_dice(context, expr, self))
            .transpose()?;
        Ok(Roll {
            total,
            success_at,
            outcome: classify(total, success_at, partial_at),
        })
    }

    /// The resist rolls this roller made, in order.
    pub(crate) fn into_reports(self) -> Vec<MoveReport> {
        self.reports
    }
}

impl expr::Dice for Roller<'_> {
    fn begin(&mut self) {
        self.key = None;
        self.drawn = 0;
    }

    fn draw(&mut self, count: u32, sides: u32) -> Result<Vec<u32>, Reason> {
        let key = self
            .key
            .get_or_insert_with(|| self.dice.next_key(self.rule, self.subject));
        let faces = faces(key, self.drawn, count, sides)?;
        self.drawn += count as usize;
        Ok(faces)
    }

    fn push_line(&mut self, text: String) {
        self.ledger.dice.push(text);
    }

    /// Rolls `check` on `actor_id`'s own keys for this rule.
    fn resist(
        &mut self,
        context: &Context,
        actor_id: &ActorId,
        check: &rule::Check,
    ) -> Result<Outcome, Reason> {
        let roll = (self.dice.roller(self.ledger, self.rule, Some(actor_id)))
            .roll_check(context, check)?;
        self.reports.push(MoveReport::Rolled {
            actor_id: actor_id.clone(),
            roll: roll.clone(),
        });
        Ok(roll.outcome)
    }
}
