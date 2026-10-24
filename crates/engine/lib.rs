//! The Spookery table engine: load a pack, run its rules, and commit or verify the journal.

pub mod canon;
pub(crate) mod diagnostic;
pub(crate) mod draft;
pub(crate) mod error;
pub(crate) mod hash;
pub(crate) mod intent;
pub(crate) mod map;
pub mod model;
pub(crate) mod round;
pub(crate) mod rules;
pub mod store;
pub(crate) mod visibility;
pub(crate) mod world;

pub(crate) use model::key;
pub(crate) use model::meter;
pub(crate) use rules::apply;
pub(crate) use rules::check;
pub(crate) use rules::effect;
pub(crate) use rules::expand;
pub(crate) use rules::perceive;
pub(crate) use rules::rule;
pub(crate) use store::commit;
pub(crate) use store::journal;
pub(crate) use store::lock;

pub use check::Outcome;
pub use diagnostic::{first_reason, reason_of, Diagnostic, Severity};
pub use error::Reason;
pub use intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
pub use map::ExitView;
pub use meter::MeterLive;
pub use perceive::{ClockView, ItemStack, KnownFact, MeterView, View};
pub use round::{
    migrate_pack, plan_catchup, play_pack_once, verify_pack, verify_quick, RoundCause, RoundOutput,
};
pub use rule::{Owner, TargetKind, When};
pub use rules::{GateRefusal, MoveReport, RollResult};
pub use visibility::RecipientLine;
pub use world::{check_pack, GmIntentSpec, Heard, QueuedMove, World, MAX_PLAYER_SEATS};

#[cfg(feature = "testing")]
pub mod testing {
    pub use crate::canon::strip_body;
    pub use crate::round::replay::from_journal;
    pub use crate::rules::roll::{faces, RollKey};

    pub mod journal {
        pub use crate::store::journal::{
            bundles, emit_entry, find_live_bundle, format_record_key, genesis_hash, gunzip_bytes,
            gzip_bytes, hash_now, latest_checkpoint, list_tick_keys, locate, read_entries,
            read_record_line, read_verified, seal, JournalEntry, TickRecord,
        };
    }

    impl crate::World {
        pub fn set_presence(&mut self, presence: crate::model::Presence) -> bool {
            self.set_presence_inner(presence)
        }

        pub fn queue_wait(
            &mut self,
            actor_id: &str,
        ) -> Result<crate::QueuedMove, crate::GateRefusal> {
            self.queue_player_move(actor_id, "wait", None)
        }
    }
}
