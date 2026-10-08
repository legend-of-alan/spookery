use cucumber::when;

use crate::World;

#[when(expr = "the docks save is checked")]
fn the_docks_save_is_checked(_world: &mut World) {
    crate::oneshot_docks::docks_save_round_trips();
}
