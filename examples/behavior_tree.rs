//! beet's behavior-tree primitives on the ESP32 — a [`Sequence`] of leaf
//! actions, driven by beet's async runtime on bare metal.
//!
//! This is the ESP32 port of beet's `examples/action/behavior_tree.rs`. The aim
//! is twofold: show the behaviour-tree control flow ([`Sequence`] runs its
//! children in order, threading the input, stopping at the first
//! [`Outcome::Fail`]), and prove beet's async <-> ECS bridge runs smoothly under
//! `no_std` + embassy.
//!
//! ## How the async runtime is wired on this board
//!
//! The upstream example uses `#[beet::main] async fn main` + `world.spawn(..).
//! call(..).await`, which is **std-only** (it owns and polls the world via
//! `AsyncRunner`). Neither is available `no_std`, so we assemble the same
//! machinery by hand:
//!
//! - [`ActionPlugin`] pulls in beet's [`AsyncPlugin`], which registers the
//!   `BeetAsyncSyncPoint` driver (in `PreUpdate`) and a [`TaskPoolPlugin`]. The
//!   driver is what grants async tasks a scoped `&mut World` once per tick.
//! - The [`Esp32Plugin`] runner already calls `app.update()` every frame, so the
//!   sync-point driver runs continuously — no manual `AsyncRunner` needed.
//! - On `no_std` there is no default [`AsyncSpawner`], so we install one backed
//!   by bevy's single-threaded [`IoTaskPool`]. This matters: the bridge driver
//!   drives woken futures by calling `tick_global_task_pools_on_main_thread()`
//!   *inline* while it holds `&mut World`, so bridge tasks must live on that pool.
//! - A `Startup` system spawns the tree and kicks off one async task that
//!   `.call()`s the root and logs the [`Outcome`].
//!
//! `Log`'s output goes through `cross_log!`, which is a no-op on `no_std`-native,
//! so the children here use a tiny defmt-logging action ([`LogStep`]) instead —
//! that way each child's execution is visible over RTT.
//!
//! Run with: `cargo run --release --no-default-features --example behavior_tree`

#![no_std]
#![no_main]

use beet::exports::bevy::tasks::IoTaskPool;
use beet::prelude::*;
use beet_esp::prelude::*;
use core::future::Future;
use core::pin::Pin;
use core::task::Context;
use core::task::Poll;
use defmt::Debug2Format;
use defmt::info;

extern crate alloc;

/// Single-thread `Send + Sync` assertion for futures handed to bevy's task pool.
///
/// bevy's single-threaded task pool still requires `spawn_local` futures to be
/// `Send + Sync` on non-wasm targets (only wasm relaxes the bound), but beet's
/// `no_std` [`AsyncSpawner`] erases tasks to a bare `Pin<Box<dyn Future>>`.
///
/// SAFETY: these futures are only ever polled on the main task (core 0) by the
/// bridge sync-point driver inside `app.update()`; they never cross to another
/// thread/core, so asserting `Send + Sync` is sound on this single-threaded
/// executor.
struct AssertSendSync<F>(F);
unsafe impl<F> Send for AssertSendSync<F> {}
unsafe impl<F> Sync for AssertSendSync<F> {}
impl<F: Future> Future for AssertSendSync<F> {
    type Output = F::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // SAFETY: structural pin projection onto the single field; we never move
        // `F` out of `self`.
        unsafe { self.map_unchecked_mut(|s| &mut s.0).poll(cx) }
    }
}

/// Leaf action: logs the caller's [`Name`] over defmt, then passes.
///
/// The defmt counterpart of beet's built-in [`Log`] (whose `cross_log!` is
/// silent on `no_std`), so the sequence's progress is observable on-device.
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

// The ECS + reflection + async-bridge + task-pool machinery has a heavy
// baseline (~139 KB at pre-startup), so the default 96 KB main heap OOMs almost
// immediately. Reclaim the over-provisioned stack region (peak use ~13 KB of
// ~210 KB) into a larger heap.
#[beet_esp::main(heap_size = 192 * 1024)]
fn main() {
    let mut app = App::new();
    app.add_plugins((Esp32Plugin, HealthPlugin, ActionPlugin));
    // no_std has no default task spawner; back beet's async bridge with bevy's
    // single-threaded IoTaskPool (initialised by AsyncPlugin's TaskPoolPlugin).
    // The bridge driver ticks this pool inline, so bridge tasks must live here.
    app.insert_resource(AsyncSpawner::new(
        |fut| {
            IoTaskPool::get().spawn_local(AssertSendSync(fut)).detach();
        },
        |fut| {
            IoTaskPool::get().spawn_local(AssertSendSync(fut)).detach();
        },
    ));
    app.add_systems(Startup, run_behavior_tree);
    app.run();
}

/// Spawns the behaviour tree and kicks off a single async task that calls the
/// root and logs the result. Mirrors the upstream example's tree shape.
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
