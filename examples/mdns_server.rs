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
//! The same mDNS task also backs the HTTP **client** resolver: a
//! `Request::get("http://<peer>.local:<port>")` from the device resolves the peer
//! over multicast. This example issues such a request against `PEER_LOCAL` (set it
//! to a `.local` name you publish on the LAN, e.g. with `avahi-publish-address`) to
//! exercise the resolver; it just logs the outcome and is harmless if no such peer
//! exists.
//!
//! The probe is a **throttled periodic retry**, not a one-shot: a one-shot fired
//! ~2s after DHCP hits the device's early-boot TCP cold-connect flakiness (the SYN
//! never reaches the peer, the GET resets). Retrying every [`PROBE_SECS`] means a
//! transient early reset is followed by a successful attempt once the stack is
//! warm. The timing lives in the *system* (an `Instant`-gated `Update`), since a
//! `run_local` task can't sleep on the bevy pool — the exact idiom
//! [`http_client`]'s `poll_example_com` established.
//!
//! Run with:
//! `cargo run --release --no-default-features --features mdns,action --example mdns_server`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_time::Instant;

extern crate alloc;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

/// The `.local` name this device advertises (without the `.local` suffix).
const HOSTNAME: &str = "beet-esp";

/// A `.local` peer the resolver test points at. Override by publishing this name
/// on the LAN (`avahi-publish -a <name>.local <ip>`) pointing at a tiny
/// `python3 -m http.server 8080`. Harmless if absent — the request just fails.
const PEER_LOCAL: &str = "http://beetpeer-test.local:8080/";

/// How often [`probe_peer`] retries the `.local` resolver GET.
const PROBE_SECS: u64 = 4;

#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::new(SSID, PASSWORD)))
        .init_resource::<Visits>()
        // beet's standard server bundle plus `MDns`: spawning it fires the
        // `on_add` hook, which starts the accept loop AND the mDNS responder
        // (advertising `beet-esp.local`) once Wi-Fi is up.
        .spawn((HttpServer::new(8080), MDns::new(HOSTNAME), Handler))
        // Throttled periodic resolver probe against a `.local` peer (PEER_LOCAL):
        // retry every PROBE_SECS so a transient early-boot connect reset is
        // followed by a successful attempt once the stack is warm.
        .add_systems(Update, probe_peer)
        .run();
}

/// Visitor counter mutated by the [`Handler`].
#[derive(Resource, Default)]
struct Visits(u32);

/// The sole request handler: a beet `Action<Request, Response>` on the server
/// entity, dispatched by `entity.exchange`.
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
        alloc::format!(
            "hello from beet-esp.local\nyou are visitor #{}\n",
            visits.0
        ),
        MediaType::Text,
    )
}

/// Exercise the `.local` resolver, throttled to [`PROBE_SECS`] (the same
/// `Instant`-gated `Update` idiom as [`http_client`]'s `poll_example_com`).
///
/// Runs every frame but only fires a request when due. The timing lives in the
/// *system* — a `run_local` task can't sleep on the bevy pool — and each request
/// is a one-shot that bridges to the background `client_driver`, whose
/// `exchange()` routes `.local` hosts through the mDNS task spawned by the `MDns`
/// server above. Retrying past the early-boot TCP cold-connect window lets a
/// transient connect reset be followed by a successful attempt.
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
                "resolver probe {} failed: {}",
                PEER_LOCAL,
                defmt::Debug2Format(&e)
            ),
        }
    });
}
