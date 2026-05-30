//! beet_router on the ESP32: route incoming HTTP requests to per-path actions.
//!
//! Where [`ecs_wifi`](../ecs_wifi.rs) serves every request through a single
//! handler system, this mirrors beet's `examples/router/router.rs` — a
//! [`router()`] dispatches each [`Request`] to the action whose path matches,
//! the same `Request -> Response` action pattern, now running `no_std` on bare
//! metal.
//!
//! ## How it routes
//!
//! [`RouterPlugin`] (beet's no_std router) registers the observers that build a
//! [`RouteTree`] from the spawned route hierarchy. Spawning an [`HttpServer`]
//! carrying [`router()`] plus the routes as children starts the accept loop
//! (via beet's `on_add` hook); each incoming request is dispatched through
//! beet's async action layer (the same `BeetAsyncSyncPoint` bridge the
//! [`behavior_tree`](../behavior_tree.rs) example drives) to the matched route's
//! action.
//!
//! ## Routes
//!
//! Once it logs its IP, hit it from the same LAN:
//!
//! ```sh
//! curl http://<device-ip>:8080/            # Home
//! curl http://<device-ip>:8080/about       # About
//! curl http://<device-ip>:8080/hello/pete  # dynamic `:name` segment
//! curl http://<device-ip>:8080/count       # increments ECS state
//! curl http://<device-ip>:8080/nope        # 404 + route listing
//! ```
//!
//! Run with: `cargo run --release --no-default-features --features router --example ecs_router`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

// ECS + reflection + async-bridge + task-pool + router + the Wi-Fi stack has a
// heavy baseline; the default 96 KiB heap OOMs. The heap and stack share one
// ~256 KiB DRAM pool, so this trades against stack headroom: 224 KiB leaves the
// stack ~32 KiB (peak request-handling use is ~19 KiB) while giving the heap
// room for per-request allocations while serving.
#[beet_esp::main(heap_size = 224 * 1024)]
fn main() {
    App::new()
        .add_plugins((
            Esp32Plugin,
            HealthPlugin,
            WifiPlugin::new(SSID, PASSWORD),
            // beet's no_std router: builds the RouteTree from the routes below.
            RouterPlugin,
        ))
        .init_resource::<Visits>()
        // Serve the router on :8080, beet-style: the `HttpServer` component plus
        // `router()` (the dispatch action) and the routes as children, each
        // binding a path to an action. Spawning it starts the accept loop.
        .spawn((HttpServer::new(8080), router(), children![
            exchange_route("", Home),
            exchange_route("about", About),
            exchange_route("hello/:name", Greet),
            exchange_route("count", Visit),
        ]))
        .run();
}

/// Visitor counter mutated by the [`Visit`] route — the ECS state a beet action
/// queries, exactly as a normal Bevy resource.
#[derive(Resource, Default)]
struct Visits(u32);

/// `GET /` — the landing page, listing the available routes.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Home(_cx: ActionContext<RequestParts>) -> Response {
    info!("route: /");
    Response::ok_body(
        "beet_esp router\n\nroutes:\n  /            this page\n  /about       about\n  /hello/:name greet by name\n  /count       visit counter\n",
        MediaType::Text,
    )
}

/// `GET /about` — a static text route.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn About(_cx: ActionContext<RequestParts>) -> Response {
    info!("route: /about");
    Response::ok_body(
        "beet_router running no_std on an ESP32-S3.\n",
        MediaType::Text,
    )
}

/// `GET /hello/:name` — reads the dynamic `:name` path segment the router
/// merged into the request params.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Greet(cx: ActionContext<RequestParts>) -> Response {
    let name = cx.input.get_param("name").unwrap_or("world");
    info!("route: /hello/{}", name);
    Response::ok_body(alloc::format!("hello, {name}!\n"), MediaType::Text)
}

/// `GET /count` — mutates ECS state, proving routes are full Bevy systems with
/// access to resources/queries.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Visit(_cx: In<ActionContext<RequestParts>>, mut visits: ResMut<Visits>) -> Response {
    visits.0 += 1;
    info!("route: /count -> {}", visits.0);
    Response::ok_body(
        alloc::format!("you are visitor #{}\n", visits.0),
        MediaType::Text,
    )
}
