//! Wi-Fi HTTP server, as a Bevy app, using beet's networking types.
//!
//! Mirrors beet's `examples/net/http_server.rs` on the ESP32: spawn beet's
//! standard [`HttpServer`] component with a single handler action as its
//! dispatch child, and every request is dispatched through it. [`WifiPlugin`]
//! brings the station up and installs the ESP32 server backend; spawning
//! `(HttpServer, CallOnReady::on_spawn(), children![Handler])` boots the accept
//! loop, exactly as a loaded `<HttpServer {CallOnReady}>` document would.
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

extern crate alloc;


#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::from_env()))
        .init_resource::<Visits>()
        .spawn((
            HttpServer::new(8080),
            CallOnReady::on_spawn(),
            children![Handler],
        ))
        .run();
}

/// Visitor counter mutated by the [`Handler`].
#[derive(Resource, Default)]
struct Visits(u32);

/// The sole request handler: a beet `Action<Request, Response>`, the server's
/// dispatch child.
#[action]
#[derive(Default, Clone, Component)]
fn Handler(cx: In<ActionContext<Request>>, mut visits: ResMut<Visits>) -> Response {
    visits.0 += 1;
    info!(
        "server request #{} -> {}",
        visits.0,
        cx.input.path_string().as_str()
    );
    Response::ok_text(alloc::format!(
        "hello from the beet_esp ECS server\nyou are visitor #{}\n",
        visits.0
    ))
}
