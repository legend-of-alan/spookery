use cucumber::when;

use crate::World;

use super::table;

#[when(expr = "the GM calls npc_act for mira")]
fn the_gm_calls_npc_act(_world: &mut World) {
    let _ = table::gm_call(2, "npc_act", r#"{"actor_id":"mira"}"#);
}
