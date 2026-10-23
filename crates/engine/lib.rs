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
#[cfg(not(any(test, feature = "testing")))]
pub(crate) use store::journal;
pub(crate) use store::lock;

pub use canon::{Owner, Rule, RuleTable, TargetKind, When};
pub use check::Outcome;
pub use diagnostic::{first_reason, reason_of, Diagnostic};
pub use error::Reason;
pub use intent::{ActorKind, Intent, IntentArgs, Source, Target, ViewerId};
pub use model::{ClockView, ExitView, ItemStack, KnownFact, MeterLive, MeterView, View};
pub use round::{
    from_journal, migrate_pack, plan_catchup, play_pack_once, verify_pack, verify_quick,
    ReplayError, ReplayReport, RoundCause, RoundOutput, VerifyError, VerifyReport,
};
pub use rules::{GateRefusal, MoveReport, RollResult};
pub use visibility::{RecipientLine, ViewerChanges};
pub use world::{check_pack, GmIntentSpec, Heard, QueuedMove, World, MAX_PLAYER_SEATS};

#[cfg(any(test, feature = "testing"))]
pub use canon::strip_body;
#[cfg(any(test, feature = "testing"))]
pub use rules::{faces, RollKey};
#[cfg(any(test, feature = "testing"))]
pub use store::journal;
