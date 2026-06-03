//! beet's behaviour-tree primitives on the ESP32: a [`Sequence`] of leaf
//! actions driven by beet's async runtime on bare metal.
//!
//! Run with: `cargo run --release --no-default-features --example behavior_tree`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::Debug2Format;
use defmt::info;

extern crate alloc;

/// Leaf action: logs the caller's [`Name`] over defmt, then passes.
#[action]
#[derive(Component)]
async fn LogStep(cx: ActionContext) -> Result<Outcome> {
    let name = cx
        .caller
        .get(|name: &Name| name.to_string())
        .await
        .unwrap_or_else(|_| "<unnamed>".to_string());
    info!("running {}", name.as_str());
    Outcome::PASS.xok()
}

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, ActionPlugin))
        .add_systems(Startup, run_behavior_tree)
        .run();
}

/// Spawns the behaviour tree and kicks off a single async task that calls the
/// root and logs the result.
fn run_behavior_tree(commands: AsyncCommands) {
    commands.run_local(async move |world: AsyncWorld| -> Result {
        let root = world
            .spawn((
                Name::new("root"),
                Sequence::new(),
                children![
                    (Name::new("child1"), LogStep),
                    (Name::new("child2"), LogStep),
                ],
            ))
            .await;

        info!("calling behavior tree root");
        let outcome = root.call::<(), Outcome>(()).await?;
        info!("sequence finished: {}", Debug2Format(&outcome));
        Ok(())
    });
}
