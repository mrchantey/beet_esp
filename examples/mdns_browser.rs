//! mDNS service **browser** over Wi-Fi, as a Bevy app.
//!
//! Where [`mdns_server`](../mdns_server.rs) *advertises* this device, this
//! example *discovers* other devices: it browses `_http._tcp.local` on the LAN
//! and logs each service as it appears and leaves.
//!
//! Spawning an [`MdnsBrowser`] entity is all it takes. Once Wi-Fi is up,
//! [`WifiPlugin`]'s mDNS wiring starts an embassy task that joins
//! `224.0.0.251:5353`, periodically multicasts a `PTR` query for the service
//! type, and feeds every inbound datagram into beet_net's **agnostic** browser
//! engine (the `UdpPacket` event -> `handle_udp_packet` observer ->
//! `DiscoveredServices` resource). That engine fires [`ServiceDiscovered`] /
//! [`ServiceRemoved`]; the [`log_discovered`] / [`log_removed`] observers below
//! print the resolved instance / host / port / addr.
//!
//! ## Try it
//!
//! From a machine on the same LAN, publish a test service and watch the device
//! discover it:
//!
//! ```sh
//! avahi-publish -s "BeetTest" _http._tcp 8080 &
//! # ...device logs `discovered: BeetTest._http._tcp.local -> <host>.local:8080`
//! kill %1   # device logs `removed: BeetTest._http._tcp.local` on the goodbye
//! ```
//!
//! Any real `_http._tcp.local` service on the LAN is discovered too. The browse
//! path is UDP multicast (proven to work on this device in phase 1), so it is not
//! subject to the phase-1 outbound-TCP-to-LAN-peer flakiness.
//!
//! Run with:
//! `cargo run --release --no-default-features --features mdns,action --example mdns_browser`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

/// The DNS-SD service type to browse for. `_http._tcp.local` enumerates HTTP
/// servers (the device's own `mdns_server` example, `python3 -m http.server`
/// behind an `avahi-publish`, printers, etc.).
const SERVICE_TYPE: &str = "_http._tcp.local";

#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::new(SSID, PASSWORD)))
        // Spawning an `MdnsBrowser` starts the browser once Wi-Fi is up: the
        // `WifiPlugin` mdns wiring sees this entity, spawns the embassy task, and
        // the agnostic engine reconciles discoveries into `DiscoveredServices`.
        .spawn(MdnsBrowser::new(SERVICE_TYPE))
        // Observe the engine's high-level events and log them.
        .add_observer(log_discovered)
        .add_observer(log_removed)
        .run();
}

/// Log each [`ServiceDiscovered`]: a service appeared (or its host/port/addr was
/// refreshed). The carried [`ServiceInstance`] has everything resolved.
fn log_discovered(ev: On<ServiceDiscovered>) {
    let inst = &ev.event().0;
    info!(
        "discovered: {} -> {}:{} (addr {})",
        defmt::Display2Format(&inst.instance),
        defmt::Display2Format(&inst.host),
        inst.port,
        defmt::Debug2Format(&inst.addr.map(|a| a.octets())),
    );
}

/// Log each [`ServiceRemoved`]: a service sent an mDNS goodbye (TTL 0) and left
/// the network.
fn log_removed(ev: On<ServiceRemoved>) {
    let inst = &ev.event().0;
    info!(
        "removed: {} ({}:{})",
        defmt::Display2Format(&inst.instance),
        defmt::Display2Format(&inst.host),
        inst.port,
    );
}
