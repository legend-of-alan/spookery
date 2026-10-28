//! Roll keys, check rolls, and expression dice for one round.

use std::collections::BTreeMap;

use crate::canon::Canon;
use crate::check::Outcome;
use crate::hash::blake2s_256;
use crate::intent::Intent;
use crate::model::Now;
use crate::round::log::RoundLog;
use crate::rule::RollSpec;
use crate::rules::expr::{eval_with_dice, Ctx, Dice};
use crate::rules::{faces_from, roll, MoveReport, RollInput, RollKey, RollResult};
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
    pub(crate) fn key(&mut self, rule: &str, subject: &str) -> RollKey {
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
            roll_index,
        }
    }

    /// The one `rules::roll` call.
    pub(crate) fn check(
        &mut self,
        canon: &Canon,
        now: &Now,
        key: RollKey,
        check_id: &str,
        target: i64,
        partial_at: Option<i64>,
    ) -> Result<RollResult, Reason> {
        let actor = now.actors.get(&key.subject_id).ok_or(Reason::UnknownKey)?;
        let viewer = now
            .actor_viewer(&key.subject_id)
            .ok_or(Reason::UnknownKey)?;
        let check = canon.checks.get(check_id).ok_or(Reason::UnknownKey)?;
        roll(
            RollInput {
                check,
                defs: &canon.meters,
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
        resist: ResistKey,
    ) -> Roller<'a> {
        Roller {
            dice: self,
            log,
            rule,
            subject,
            resist,
            key: None,
            drawn: 0,
            reports: Vec::new(),
        }
    }
}

/// Which name seeds a resist roll. Kept per site so keys stay byte-identical.
#[derive(Clone, Copy)]
pub(crate) enum ResistKey {
    Rule,
    Check,
}

/// Expression dice and resist rolls for one rule and subject. `finish` is the only way out.
pub(crate) struct Roller<'a> {
    dice: &'a mut RoundDice,
    log: &'a mut RoundLog,
    rule: &'a str,
    subject: &'a str,
    resist: ResistKey,
    key: Option<RollKey>,
    drawn: usize,
    reports: Vec<MoveReport>,
}

impl Roller<'_> {
    /// Evaluate a rule's `roll:` spec: target, then partial_at, then the check keyed by `rule_key`.
    pub(crate) fn roll_spec(
        &mut self,
        canon: &Canon,
        now: &Now,
        intent: &Intent,
        spec: &RollSpec,
        rule_key: &str,
    ) -> Result<RollResult, Reason> {
        let ctx = Ctx { intent };
        let target = eval_with_dice(canon, now, &ctx, &spec.target, self)?.value;
        let partial_at = match &spec.partial_at {
            Some(expr) => Some(eval_with_dice(canon, now, &ctx, expr, self)?.value),
            None => None,
        };
        let key = self.dice.key(rule_key, &intent.actor_id);
        self.dice
            .check(canon, now, key, &spec.check, target, partial_at)
    }

    /// The resist rolls this roller made, in order. Consumes the roller and ends its borrow.
    pub(crate) fn finish(self) -> Vec<MoveReport> {
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
            .get_or_insert_with(|| self.dice.key(self.rule, self.subject));
        let faces = faces_from(key, self.drawn, count, sides)?;
        self.drawn += count as usize;
        Ok(faces)
    }

    fn note(&mut self, text: String) {
        self.log.dice.push(text);
    }

    fn resist(
        &mut self,
        canon: &Canon,
        now: &Now,
        actor_id: &str,
        check_id: &str,
        target: i64,
    ) -> Result<Outcome, Reason> {
        let rule = match self.resist {
            ResistKey::Rule => self.rule,
            ResistKey::Check => check_id,
        };
        let key = self.dice.key(rule, actor_id);
        let out = self.dice.check(canon, now, key, check_id, target, None)?;
        self.reports.push(MoveReport::Rolled {
            actor_id: actor_id.to_string(),
            out: out.clone(),
        });
        Ok(out.outcome)
    }
}
