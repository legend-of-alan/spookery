//! The pack kernel: load a world, run its rules, and commit the save.
//!
//! Public API
//! - `api`: the only names other crates may use.
//!
//! Internal
//! - `canon`: the loaded pack.
//! - `diagnostic`: one problem a loader or pack check can show.
//! - `draft`: the staged save a round writes before commit.
//! - `error`: failure tokens. JSONL lowers `BUDGET` at the log edge.
//! - `firewall`: whether one viewer may see a fact.
//! - `format`: format tags, one const per file.
//! - `hash`: Blake2s, as raw bytes or as hex.
//! - `host_summary`: the host digest, and the player digest built from the view.
//! - `intent`: who is asking, and the action they asked the round to take.
//! - `map`: exits and the journeys that follow them.
//! - `model`: the live save and the keys a commit watches.
//! - `pause`: pause plot clocks while the table is empty.
//! - `rules`: conditions, costs, gates, rolls, and effects.
//! - `speech`: who hears a say. The host writes the tape.
//! - `store`: the save, the journal, and the rules version stamped on both.
//! - `visibility`: what one viewer may hear from a round.
//! - `world`: an open pack, and the check that reads one without writing.

pub mod api;

pub(crate) mod canon;
pub(crate) mod diagnostic;
pub(crate) mod draft;
pub(crate) mod error;
pub(crate) mod firewall;
pub(crate) mod format;
pub(crate) mod hash;
pub(crate) mod host_summary;
pub(crate) mod intent;
pub(crate) mod map;
pub(crate) mod model;
pub(crate) mod pause;
pub(crate) mod rules;
pub(crate) mod speech;
pub(crate) mod store;
pub(crate) mod visibility;
pub(crate) mod world;

pub(crate) use model::ids;
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
