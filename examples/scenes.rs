//! Scene-driven firmware: the device's API is *sent to it* as a beet scene, not
//! baked in.
//!
//! The firmware ships a tiny bootstrap server with only meta-routes — `load`,
//! `clear`, `reset`, `dump` — and loads its real routes over the wire. A scene
//! (a reflection-serialized slice of the ECS) is POSTed to `/load`; from that
//! moment the scene's routes *are* the API. The behaviours it wires
//! ([`ActionRoute`], the behaviour-tree leaves, rhai [`Script`]s) all live in
//! [`beet_esp::scene`]; the scene supplies only which behaviour sits at which
//! path.
//!
//! This runs on a bare ESP32 breakout: the on-board WS2812 is driven by a rhai
//! `Script` (the `led-script` scene reads the elapsed tick + current colour and
//! returns the next colour, keeping a counter in its persistent `state` map).
//! Built with `--features alvik` it also brings up the Alvik robot, adding its
//! drive/led/dance/line-follower/roomba/script scenes.
//!
//! ```sh
//! curl http://192.168.86.222:8080/                 # this help + current routes
//! curl http://192.168.86.222:8080/dump             # current scene as JSON
//! curl --data-binary @scene.json \
//!      -H 'content-type: application/json' \
//!      http://192.168.86.222:8080/load             # load a scene
//! curl http://192.168.86.222:8080/clear            # despawn scene + reset
//! curl http://192.168.86.222:8080/reset            # stop hardware
//! ```
//!
//! On boot the firmware logs canonical example scenes over defmt; save one to
//! `scene.json` to try `/load`.
//!
//! Run with (bare ESP32):
//! `cargo run --release --no-default-features --features led,router,rhai --example scenes`
//!
//! Run with (Alvik):
//! `cargo run --release --no-default-features --features alvik,router,rhai --example scenes`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

/// Static IPv4 the device binds to, so a controller reaches it at a fixed
/// address with no lookup.
const SCENE_IP: [u8; 4] = [192, 168, 86, 222];

#[beet_esp::main]
fn main() {
    let mut app = App::new();
    app.add_plugins((
        Esp32Plugin,
        HealthPlugin,
        WifiPlugin::from_env().with_static_ip(SCENE_IP),
        RouterPlugin,
        SceneServerPlugin,
    ));

    // The Alvik build drives the robot; the bare build drives the on-board
    // WS2812. Both expose `LedColor` entities, so the generic scene routes and
    // the reset handler work either way.
    #[cfg(feature = "alvik")]
    app.add_plugins(AlvikPlugin);
    #[cfg(not(feature = "alvik"))]
    {
        app.add_plugins(LedPlugin);
        app.spawn((LedColor::default(), Ws2812Led));
    }

    // The bootstrap server: only the meta-routes. The real routes arrive via
    // `/load`. `SceneRoot`s get reparented here and picked up by the router.
    app.add_systems(Startup, log_scenes)
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

/// Log the canonical example scenes over defmt on boot, so each can be saved and
/// POSTed to `/load`.
#[allow(unused_variables)]
fn log_scenes(world: &mut World) {
    #[cfg(all(feature = "rhai", feature = "led"))]
    log_scene(world, "led-script", led_script_scene());
    #[cfg(feature = "alvik")]
    log_alvik_scenes(world);
}
