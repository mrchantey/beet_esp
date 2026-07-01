//! Wi-Fi WebSocket client as a Bevy app, using beet's socket types.
//!
//! [`WifiPlugin`] joins the AP named by the `WIFI_SSID`/`WIFI_PASSWORD` env vars
//! and installs the esp `Socket::connect` transport. On `Startup` a [`Socket`]
//! connects to `SOCKET_SERVER` (from the local `.env`); on [`SocketReady`] it
//! sends a text message and logs every message the server echoes back, then
//! closes cleanly when the echo returns.
//!
//! Start the host echo server first (`beet socket-server`), then:
//! `cargo run --release --no-default-features --features wifi,action,sockets --example socket_client`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet::prelude::sockets::*;
// disambiguate the socket `Message` enum from bevy's `Message` trait.
use beet::prelude::sockets::Message;
use beet_esp::prelude::*;

/// The `host:port` the device connects to, from the local `.env` (`build.rs`
/// forwards it as a `cargo:rustc-env`).
const SOCKET_SERVER: &str = env!("SOCKET_SERVER");

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((Esp32Plugin, HealthPlugin, WifiPlugin::from_env()))
        .add_systems(Startup, connect_socket)
        .run();
}

/// Spawn the socket entity, connecting to `SOCKET_SERVER`, with its ready/recv
/// observers.
fn connect_socket(mut commands: Commands) {
    info!("connecting websocket to `{}`", SOCKET_SERVER);
    commands.spawn((
        Socket::insert_on_connect(SOCKET_SERVER),
        OnSpawn::observe(on_ready),
        OnSpawn::observe(on_recv),
    ));
}

/// On connect, greet the server.
fn on_ready(ev: On<SocketReady>, mut commands: Commands) {
    info!("socket ready; sending greeting");
    commands
        .entity(ev.target())
        .trigger_target(MessageSend(Message::text("gday from esp32")));
}

/// Log each echoed message; close once our greeting returns.
fn on_recv(ev: On<MessageRecv>, mut commands: Commands) {
    match ev.event().inner() {
        Message::Text(text) => {
            info!("echo received: {}", text);
            commands
                .entity(ev.target())
                .trigger_target(MessageSend(Message::Close(None)));
        }
        Message::Close(_) => info!("socket closed cleanly"),
        other => info!("recv: {:?}", other),
    }
}
