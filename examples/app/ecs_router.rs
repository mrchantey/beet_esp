//! beet_router on the ESP32: route incoming HTTP requests to per-path actions.
//!
//! [`RouterPlugin`] builds a [`RouteTree`] from the spawned route hierarchy;
//! spawning an [`HttpServer`] with [`default_router()`] and the routes as
//! `children![..]` starts the accept loop and dispatches each [`Request`] to the
//! matching action. Once it logs its IP, hit it from the same LAN:
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

extern crate alloc;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((
            Esp32Plugin,
            HealthPlugin,
            WifiPlugin::from_env(),
            RouterPlugin,
        ))
        .init_resource::<Visits>()
        .spawn((
            HttpServer::new(8080),
            default_router(),
            children![
                exchange_route("", Home),
                exchange_route("about", About),
                exchange_route("hello/:name", Greet),
                exchange_route("count", Visit),
            ],
        ))
        .run();
}

/// Visitor counter mutated by the [`Visit`] route.
#[derive(Resource, Default)]
struct Visits(u32);

/// `GET /` — the landing page, listing the available routes.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Home(_cx: ActionContext<RequestParts>) -> Response {
    info!("route: /");
    Response::ok_text(
        "beet_esp router\n\nroutes:\n  /            this page\n  /about       about\n  /hello/:name greet by name\n  /count       visit counter\n",
    )
}

/// `GET /about` — a static text route.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn About(_cx: ActionContext<RequestParts>) -> Response {
    info!("route: /about");
    Response::ok_text("beet_router running no_std on an ESP32-S3.\n")
}

/// `GET /hello/:name` — reads the dynamic `:name` path segment.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Greet(cx: ActionContext<RequestParts>) -> Response {
    let name = cx.input.get_param("name").unwrap_or("world");
    info!("route: /hello/{}", name);
    Response::ok_text(alloc::format!("hello, {name}!\n"))
}

/// `GET /count` — mutates ECS state, proving routes are full Bevy systems.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Visit(_cx: In<ActionContext<RequestParts>>, mut visits: ResMut<Visits>) -> Response {
    visits.0 += 1;
    info!("route: /count -> {}", visits.0);
    Response::ok_text(alloc::format!("you are visitor #{}\n", visits.0))
}
