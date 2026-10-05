#[path = "support.rs"]
#[allow(dead_code)]
mod support;

#[path = "steps/harness.rs"]
mod harness;

use std::cell::RefCell;
use std::process::Child;

use cucumber::World as _;

std::thread_local! {
    static CHILDREN: RefCell<Vec<Child>> = const { RefCell::new(Vec::new()) };
}

/// Scenarios that spawn `spook` register the child here.
/// The after hook drops every child, including when the scenario fails.
#[allow(dead_code)]
pub fn track_child(child: Child) {
    CHILDREN.with(|slot| slot.borrow_mut().push(child));
}

fn drop_children() {
    CHILDREN.with(|slot| {
        for mut child in slot.borrow_mut().drain(..) {
            let _ = child.kill();
            let _ = child.wait();
        }
    });
}

#[derive(Debug, Default, cucumber::World)]
pub struct World {
    pub pack: Option<std::path::PathBuf>,
    pub scratch: String,
    pub before: Vec<(std::path::PathBuf, Vec<u8>)>,
    pub code: Option<i32>,
}

#[tokio::main]
async fn main() {
    World::cucumber()
        .max_concurrent_scenarios(1)
        .fail_on_skipped()
        .after(|_, _, _, _, world| {
            drop_children();
            if let Some(world) = world {
                if let Some(pack) = world.pack.take() {
                    let _ = std::fs::remove_dir_all(pack);
                }
            }
            Box::pin(async {})
        })
        .run_and_exit("tests/features")
        .await;
}
