//! Scene-driven Alvik: the robot's API is *sent to it* as a beet scene, not
//! baked into the firmware.
//!
//! The firmware ships a tiny bootstrap server with only meta-routes — `load`,
//! `clear`, `reset`, `dump` — and loads its real routes over the wire. A scene
//! (a reflection-serialized slice of the ECS) is POSTed to `/load`; from that
//! moment the scene's routes *are* the API. The behaviours it wires
//! ([`DriveRoute`], [`ActionRoute`], the behaviour-tree leaves, the rhai
//! [`AlvikScript`](beet_esp::alvik::scripting)) all live in
//! [`beet_esp::alvik`]; the scene supplies only which behaviour sits at which
//! path.
//!
//! ```sh
//! curl http://192.168.86.222:8080/                 # this help + current routes
//! curl http://192.168.86.222:8080/dump             # current scene as JSON
//! curl --data-binary @scene.json \
//!      -H 'content-type: application/json' \
//!      http://192.168.86.222:8080/load             # load a scene
//! curl http://192.168.86.222:8080/clear            # despawn scene + reset
//! curl http://192.168.86.222:8080/reset            # stop motors, leds off
//! ```
//!
//! On boot the firmware logs canonical example scenes over defmt; save one to
//! `scene.json` to try `/load`.
//!
//! Run with: `cargo run --release --no-default-features --features alvik,router --example alvik-scene`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

/// Static IPv4 the robot binds to (matches [`rc`](./rc.rs) so the same
/// controller reaches it).
const ALVIK_IP: [u8; 4] = [192, 168, 86, 222];

#[beet_esp::main]
fn main() {
    App::new()
        .add_plugins((
            Esp32Plugin,
            HealthPlugin,
            WifiPlugin::from_env().with_static_ip(ALVIK_IP),
            AlvikPlugin,
            RouterPlugin,
            AlvikScenePlugin,
        ))
        .add_systems(Startup, log_example_scenes)
        // The bootstrap server: only the meta-routes. The real routes arrive via
        // `/load`. `SceneRoot`s get reparented here and picked up by the router.
        .spawn((
            HttpServer::new(8080),
            default_router(),
            children![
                exchange_route("", Home),
                exchange_route("load", LoadScene),
                exchange_route("clear", ClearScene),
                exchange_route("reset", Reset),
                exchange_route("dump", DumpScene),
            ],
        ))
        .run();
}
