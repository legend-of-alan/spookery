//! Write the round line and commit a live round.

use crate::session;
use crate::store::records::RoundRecord;
use crate::store::{self, RULES_REV};
use crate::Reason;

use crate::ecs::Presence;
use crate::round::stages::{self, deliver::Deliveries};
use crate::round::{self, Ledger};

/// What the round showed. A live round adds its commit outcome.
pub(crate) fn take_output(ledger: &mut Ledger, delivered: Deliveries) -> round::Output {
    round::Output {
        reports: std::mem::take(&mut ledger.reports),
        dice: ledger.dice.clone(),
        changed_keys: delivered.changed_keys,
        lines: delivered.lines,
        seen: delivered.seen,
        effects: ledger.effects.clone(),
        commit: None,
    }
}

/// Write the round line and commit it. The session owns the pack lock.
pub(crate) fn run(
    session: &mut session::Session,
    staged: stages::Output,
    presence: &Presence,
    wall: &str,
    fast_forward: bool,
) -> Result<round::Output, Reason> {
    let stages::Output {
        world,
        clock,
        mut ledger,
        delivered,
    } = staged;
    let mut output = take_output(&mut ledger, delivered);
    let record = RoundRecord {
        round_index: clock.round_index,
        rules_rev: RULES_REV,
        state_rev_before: 0,
        state_rev_after: 0,
        world_time_before: clock.world_time_before,
        world_time_after: clock.world_time_after,
        presence_players: presence.players,
        actors_present: presence.actors_present.clone(),
        last_round_at: wall.to_string(),
        intents: session.queued_intents().to_vec(),
        refused: ledger.refused,
        effects: ledger.effects,
        fast_forward,
        dice: ledger.dice,
        extra_steps: ledger.extra_steps,
        memories: Vec::new(),
        state_hash: String::new(),
        prev: String::new(),
        chain: String::new(),
    };
    output.commit = Some(store::commit_round(session, world, record)?);
    Ok(output)
}
