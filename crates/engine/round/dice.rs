//! Roll keys and expression dice for one round.

use std::collections::BTreeMap;

use crate::ecs::id::ActorId;
use crate::hash::blake2s_256;
use crate::round::Ledger;
use crate::rules::expr::eval;
use crate::rules::roll::{self, faces};
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

    pub(crate) fn roller<'r>(
        &'r mut self,
        ledger: &'r mut Ledger,
        rule: &'r str,
        subject: Option<&'r ActorId>,
    ) -> Roller<'r> {
        Roller {
            dice: self,
            ledger,
            rule,
            subject,
            key: None,
            drawn: 0,
        }
    }
}

/// Expression dice for one rule and subject, all keyed by the rule id.
pub(crate) struct Roller<'r> {
    dice: &'r mut Dice,
    ledger: &'r mut Ledger,
    rule: &'r str,
    subject: Option<&'r ActorId>,
    key: Option<roll::Key>,
    drawn: usize,
}

impl eval::Dice for Roller<'_> {
    fn start(&mut self) {
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
}
