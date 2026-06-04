//! Wiring beet's async <-> ECS bridge onto this board's executor.
//!
//! beet's [`action`](beet::action) layer (and anything else built on the
//! `beet_async` bridge) lets async tasks touch the Bevy [`World`]. A task can't
//! grab `&mut World` whenever it likes; instead it enqueues a request that the
//! `BeetAsyncSyncPoint` driver services once per `app.update()`. The driver
//! publishes a scoped `&mut World`, wakes the queued futures, **polls them
//! inline**, then unpublishes the pointer. Async actions never `.await` on
//! hardware — they `.await` on *world access*, which only exists during that
//! window.
//!
//! On `no_std` there is no default task spawner ([`AsyncSpawner::default`] panics),
//! so the host app must install one. This module is that installer.
//!
//! ## Why bevy's task pool and not embassy
//!
//! It comes down to *who polls the bridge future, and when*. On `no_std` the
//! driver's internal wait is a no-op: correctness relies entirely on the woken
//! futures being polled **synchronously, on the driver's own call stack, while
//! the world pointer is still published**. The only thing the driver pumps in
//! that window is bevy's global task pools — via
//! `tick_global_task_pools_on_main_thread()`. So a bridge task *must* live on
//! that executor.
//!
//! Spawning a bridge task on embassy instead would dead-lock:
//!
//! - The driver wakes the task's waker, but then ticks the *bevy* pool, not
//!   embassy — so the embassy task isn't polled.
//! - The driver itself runs *inside* the embassy task that runs `app.update()`
//!   (see [`Esp32Plugin`](crate::esp32_plugin)), and never yields mid-sync-point.
//!   embassy can't poll the sibling bridge task until `app.update()` returns and
//!   the frame timer awaits — by which point the world pointer is unpublished.
//! - The task finally polls, finds no published world, re-enqueues, returns
//!   `Pending`. It only ever runs *outside* the access window. Live-lock.
//!
//! It's a scheduling-window problem, not a capability one. Note also that
//! `IoTaskPool` is a misnomer here: on `no_std` there are **no threads** — the
//! three bevy pools all alias one global `edge_executor` (the embedded-friendly,
//! synchronously-*tickable* executor). That tickability is exactly what the
//! bridge needs, and exactly what embassy's interrupt-driven run-loop is not
//! built to offer. So embassy and this pool aren't competing: embassy drives the
//! things that await silicon (the LED/Wi-Fi drivers, and `app.update()` itself
//! — see [`async_bridge`](crate::esp32_utils::async_bridge)); this pool drives the things that await the
//! `World`.
//!
//! The one wrinkle is [`AssertSendSync`]: bevy keys its `spawn_local` `Send +
//! Sync` bound off `target_arch == "wasm32"`, not off actual threading, so on a
//! non-wasm single-core target the bound is spurious. We assert past it (sound,
//! since these futures are only ever polled on core 0 — see the SAFETY note).

use beet::exports::bevy::tasks::IoTaskPool;
use beet::prelude::*;
use core::future::Future;
use core::pin::Pin;
use core::task::Context;
use core::task::Poll;

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

/// Install the [`AsyncSpawner`] that backs beet's async bridge with bevy's
/// single-threaded `IoTaskPool` (initialised by `AsyncPlugin`'s `TaskPoolPlugin`).
///
/// `insert_resource` overrides the panicking `no_std` default regardless of
/// plugin order. Called by [`Esp32Plugin`](crate::esp32_plugin) when the
/// `action` feature is enabled. See the module docs for why this pool and not
/// embassy.
pub fn install_async_spawner(app: &mut App) {
    app.insert_resource(AsyncSpawner::new(
        |fut| {
            IoTaskPool::get().spawn_local(AssertSendSync(fut)).detach();
        },
        |fut| {
            IoTaskPool::get().spawn_local(AssertSendSync(fut)).detach();
        },
    ));
}
