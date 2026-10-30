//! Roll keys, check rolls, and expression dice for one round.

use std::collections::BTreeMap;

use crate::canon::Canon;
use crate::effect::Effect;
use crate::hash::blake2s_256;
use crate::intent::Intent;
use crate::model::{Presence, State};
use crate::round::log::RoundLog;
use crate::rule::RollSpec;
use crate::rules::expr::{eval_with_dice, Ctx, Dice};
use crate::rules::roll::Outcome;
use crate::rules::{faces_from, roll_check, MoveReport, Roll, RollInput, RollKey};
use crate::Reason;

/// The round line's `seed` field. A roll uses `RollKey`, not this string.
pub(crate) fn round_seed(world_id: &str, round_index: u64) -> String {
    format!("{world_id}|{round_index}")
}

pub(crate) fn shuffle_key(world_id: &str, round_index: u64, actor_id: &str) -> [u8; 32] {
    blake2s_256(format!("{world_id}|{round_index}|{actor_id}").as_bytes())
}

/// Every roll key of one round.
pub(crate) struct RoundDice {
    world_id: String,
    round_index: u64,
    counts: BTreeMap<(String, String), u32>,
}

impl RoundDice {
    pub(crate) fn new(world_id: String, round_index: u64) -> Self {
        Self {
            world_id,
            round_index,
            counts: BTreeMap::new(),
        }
    }

    /// The one allocator: the next key for this rule and subject.
    pub(crate) fn next_key(&mut self, rule: &str, subject: &str) -> RollKey {
        let index = self
            .counts
            .entry((rule.to_string(), subject.to_string()))
            .or_insert(0);
        let roll_index = *index;
        *index += 1;
        RollKey {
            world_id: self.world_id.clone(),
            round_index: self.round_index,
            rule_id: rule.to_string(),
            subject_id: subject.to_string(),
            index: roll_index,
        }
    }

    /// The one `rules::roll` call.
    pub(crate) fn roll_check(
        &mut self,
        canon: &Canon,
        now: &State,
        key: RollKey,
        check_id: &str,
        target: i64,
        partial_at: Option<i64>,
    ) -> Result<Roll, Reason> {
        let actor = now.actors.get(&key.subject_id).ok_or(Reason::UnknownId)?;
        let viewer = actor.viewer(&key.subject_id);
        let check = canon.checks.get(check_id).ok_or(Reason::UnknownId)?;
        roll_check(
            RollInput {
                check,
                meters: &canon.meters,
                actor_id: &key.subject_id,
                actor,
                viewer: &viewer,
                target,
                partial_at,
            },
            &key,
        )
    }

    pub(crate) fn roller<'a>(
        &'a mut self,
        log: &'a mut RoundLog,
        rule: &'a str,
        subject: &'a str,
    ) -> Roller<'a> {
        Roller {
            dice: self,
            log,
            rule,
            subject,
            key: None,
            drawn: 0,
            reports: Vec::new(),
        }
    }
}

/// What a rule's effects came to: the effects, the roll that picked them, and the resist rolls made on the way.
pub(crate) struct Expansion {
    pub effects: Vec<Effect>,
    pub rolled: Option<Roll>,
    pub resisted: Vec<MoveReport>,
}

/// Expression dice and resist rolls for one rule and subject, all keyed by the rule id.
/// `finish` returns the resist reports. Dropping the roller discards them.
pub(crate) struct Roller<'a> {
    dice: &'a mut RoundDice,
    log: &'a mut RoundLog,
    rule: &'a str,
    subject: &'a str,
    key: Option<RollKey>,
    drawn: usize,
    reports: Vec<MoveReport>,
}

impl Roller<'_> {
    /// Evaluate a rule's `roll:` spec, then roll its check.
    pub(crate) fn roll_spec(
        &mut self,
        canon: &Canon,
        now: &State,
        presence: &Presence,
        intent: &Intent,
        spec: &RollSpec,
    ) -> Result<Roll, Reason> {
        let (target, partial_at) = self.eval_thresholds(canon, now, presence, intent, spec)?;
        let key = self.dice.next_key(self.rule, intent.actor.actor_id());
        self.dice
            .roll_check(canon, now, key, &spec.check, target, partial_at)
    }

    /// The one order a `roll:` spec's thresholds draw dice in: target, then partial_at.
    pub(crate) fn eval_thresholds(
        &mut self,
        canon: &Canon,
        now: &State,
        presence: &Presence,
        intent: &Intent,
        spec: &RollSpec,
    ) -> Result<(i64, Option<i64>), Reason> {
        let ctx = Ctx { intent, presence };
        let target = eval_with_dice(canon, now, &ctx, &spec.target, self)?.value;
        let partial_at = match &spec.partial_at {
            Some(expr) => Some(eval_with_dice(canon, now, &ctx, expr, self)?.value),
            None => None,
        };
        Ok((target, partial_at))
    }

    /// The resist rolls this roller made, in order. Consumes the roller and ends its borrow.
    pub(crate) fn into_reports(self) -> Vec<MoveReport> {
        self.reports
    }
}

impl Dice for Roller<'_> {
    fn begin(&mut self) {
        self.key = None;
        self.drawn = 0;
    }

    fn draw(&mut self, count: u32, sides: u32) -> Result<Vec<u32>, Reason> {
        let key = self
            .key
            .get_or_insert_with(|| self.dice.next_key(self.rule, self.subject));
        let faces = faces_from(key, self.drawn, count, sides)?;
        self.drawn += count as usize;
        Ok(faces)
    }

    fn log(&mut self, text: String) {
        self.log.dice.push(text);
    }

    fn resist(
        &mut self,
        canon: &Canon,
        now: &State,
        actor_id: &str,
        check_id: &str,
        target: i64,
    ) -> Result<Outcome, Reason> {
        let key = self.dice.next_key(self.rule, actor_id);
        let out = self
            .dice
            .roll_check(canon, now, key, check_id, target, None)?;
        self.reports.push(MoveReport::Rolled {
            actor_id: actor_id.to_string(),
            roll: out.clone(),
        });
        Ok(out.outcome)
    }
}
