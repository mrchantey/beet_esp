//! Wi-Fi HTTP server, as a Bevy app, using beet's networking types.
//!
//! Mirrors beet's `examples/net/http_server.rs` on the ESP32: spawn beet's
//! standard [`HttpServer`] component alongside an [`exchange_handler`]-style
//! action, and every request is dispatched through it. [`WifiPlugin`] brings the
//! station up and installs the ESP32 server backend; spawning `(HttpServer,
//! Handler)` drives the rest via beet's `on_add` hook.
//!
//! Each request runs [`Handler`] on the ECS — a full Bevy system with access to
//! resources — and returns a beet [`Response`]. Hit it from the same LAN once it
//! logs its IP:
//!
//! ```sh
//! curl http://<device-ip>:8080
//! ```
//!
//! For path-based routing to many actions, see [`ecs_router`](../ecs_router.rs).
//!
//! Run with: `cargo run --release --no-default-features --features wifi,action --example http_server`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;


// The World/registry/request bulk lives in PSRAM now (see `beet_esp::mem`), so
// internal SRAM only has to hold the radio + DMA + stack. The default 64 KiB
// internal reserve (+ the ~72 KiB reclaimed region) is plenty; the stack keeps
// the rest of the DRAM pool (peak use ~19 KiB while serving).
#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::from_env()))
        .init_resource::<Visits>()
        // beet's standard server bundle: the `HttpServer` component plus its
        // `Action<Request, Response>` handler on the same entity. Spawning it
        // fires the `on_add` hook, which starts the accept loop once Wi-Fi is up.
        .spawn((HttpServer::new(8080), Handler))
        .run();
}

/// Visitor counter mutated by the [`Handler`] — the ECS state a beet action
/// queries, exactly as a normal Bevy resource.
#[derive(Resource, Default)]
struct Visits(u32);

/// The sole request handler: a beet `Action<Request, Response>` on the server
/// entity, dispatched by `entity.exchange`. A full Bevy system, so it can read
/// and mutate ECS state.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Handler(cx: In<ActionContext<Request>>, mut visits: ResMut<Visits>) -> Response {
    visits.0 += 1;
    info!(
        "server request #{} -> {}",
        visits.0,
        cx.input.path_string().as_str()
    );
    Response::ok_body(
        alloc::format!("hello from the beet_esp ECS server\nyou are visitor #{}\n", visits.0),
        MediaType::Text,
    )
}
