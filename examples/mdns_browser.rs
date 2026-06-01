//! mDNS service browser on the ESP32: discover other HTTP servers on the LAN.
//!
//! Spawns an [`MdnsBrowser`] for `_http._tcp.local`; [`WifiPlugin`]'s mDNS wiring
//! starts an embassy task that joins the multicast group, queries, and feeds
//! responses into beet_net's agnostic browser engine. As services come and go,
//! the engine spawns one entity per service carrying an [`MDnsService`] component
//! and despawns it on a goodbye; here we log those via `Add`/`Remove` observers.
//!
//! Run with: `cargo run --release --no-default-features --features mdns --example mdns_browser`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;

#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::from_env()))
        // React to services appearing/disappearing as `MDnsService` entities are
        // spawned/despawned by the agnostic browser engine.
        .add_observer(on_discovered)
        .add_observer(on_removed)
        .spawn(MdnsBrowser::http())
        .run();
}

/// Log a service as its `MDnsService` entity is spawned.
fn on_discovered(ev: On<Add, MDnsService>, services: Query<&MDnsService>) -> Result {
    let service = services.get(ev.entity)?;
    info!(
        "discovered: {} at {}:{}",
        service.instance.as_str(),
        service.host.as_str(),
        service.port
    );
    Ok(())
}

/// Log a service as its `MDnsService` entity is despawned.
fn on_removed(ev: On<Remove, MDnsService>, services: Query<&MDnsService>) -> Result {
    let service = services.get(ev.entity)?;
    info!("removed: {}", service.instance.as_str());
    Ok(())
}
