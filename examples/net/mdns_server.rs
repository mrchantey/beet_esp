//! Wi-Fi HTTP server advertised over mDNS, as a Bevy app.
//!
//! Same shape as [`http_server`](../http_server.rs), but the server entity also
//! carries an [`MDns`] component, so once Wi-Fi is up the device claims
//! `beet-esp.local` on the LAN and answers multicast `A` queries for it. From any
//! machine on the same network:
//!
//! ```sh
//! ping beet-esp.local
//! curl http://beet-esp.local:8080/
//! ```
//!
//! The same mDNS task also backs the HTTP **client** resolver: this example
//! issues a throttled periodic GET against `PEER_LOCAL` (a `.local` name you
//! publish on the LAN) to exercise it, logging the outcome and harmless if no
//! such peer exists.
//!
//! Run with:
//! `cargo run --release --no-default-features --features mdns,action --example mdns_server`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use embassy_time::Instant;

extern crate alloc;


/// The `.local` name this device advertises (without the `.local` suffix).
const HOSTNAME: &str = "beet-esp";

/// A `.local` peer the resolver test points at. Override by publishing this name
/// on the LAN (`avahi-publish -a <name>.local <ip>`) pointing at a tiny
/// `python3 -m http.server 8080`. Harmless if absent — the request just fails.
const PEER_LOCAL: &str = "http://beetpeer-test.local:8080/";

/// How often [`probe_peer`] retries the `.local` resolver GET.
const PROBE_SECS: u64 = 4;

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::from_env()))
        .init_resource::<Visits>()
        .spawn((HttpServer::new(8080), BootOnLoad, MDns::new(HOSTNAME), Handler))
        .add_systems(Update, probe_peer)
        .run();
}

/// Visitor counter mutated by the [`Handler`].
#[derive(Resource, Default)]
struct Visits(u32);

/// The sole request handler.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Handler(cx: In<ActionContext<Request>>, mut visits: ResMut<Visits>) -> Response {
    visits.0 += 1;
    info!(
        "server request #{} -> {}",
        visits.0,
        cx.input.path_string().as_str()
    );
    Response::ok_text(alloc::format!(
        "hello from beet-esp.local\nyou are visitor #{}\n",
        visits.0
    ))
}

/// Exercise the `.local` resolver, throttled to [`PROBE_SECS`].
fn probe_peer(commands: AsyncCommands, mut last_tick: Local<Option<Instant>>) {
    let now = Instant::now();
    let due = last_tick.is_none_or(|t| (now - t).as_secs() >= PROBE_SECS);
    if !due {
        return;
    }
    *last_tick = Some(now);
    commands.run_local(async move |_world: AsyncWorld| {
        match Request::get(PEER_LOCAL).send().await {
            Ok(response) => info!(
                "resolver probe {} -> status {}",
                PEER_LOCAL,
                response.status().as_u16()
            ),
            Err(e) => warn!(
                "resolver probe {} failed: {:?}",
                PEER_LOCAL,
                e
            ),
        }
    });
}
