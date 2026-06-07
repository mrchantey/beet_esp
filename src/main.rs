//! Scene-driven firmware: the device's API is *sent to it* as a beet scene, not
//! baked in. This is the default binary for the crate — `cargo run` flashes it.
//!
//! The firmware ships a tiny bootstrap server with only meta-routes — `load`,
//! `clear`, `reset`, `dump` — and loads its real routes over the wire. A scene
//! (a reflection-serialized slice of the ECS) is POSTed to `/load`; from that
//! moment the scene's routes *are* the API. The behaviours it wires
//! ([`SpawnAction`], the behaviour-tree leaves, rhai [`Script`]s) live upstream
//! in [`beet::router`] and in [`beet_esp::scene`]; the scene supplies only which
//! behaviour sits at which path.
//!
//! This runs on a bare ESP32 breakout: the on-board WS2812 is driven by a rhai
//! `Script` (the `led-script` scene reads the elapsed tick + current colour and
//! returns the next colour, keeping a counter in its persistent `state` map).
//! Built with `--features alvik` it also brings up the Alvik robot, adding its
//! drive/led/dance/line-follower/roomba/script scenes.
//!
//! ```sh
//! curl http://192.168.86.222:8337/                 # this help + current routes
//! curl http://192.168.86.222:8337/dump             # current scene as JSON
//! curl --data-binary @scene.json \
//!      -H 'content-type: application/json' \
//!      http://192.168.86.222:8337/load             # load a scene
//! curl http://192.168.86.222:8337/clear            # despawn scene + reset
//! curl http://192.168.86.222:8337/reset            # stop hardware
//! ```
//!
//! Or drive it from the host with the upstream `beet` CLI:
//! `beet load scenes/led-script.json` (with `BEET_REMOTE_URL` set to the device).
//!
//! The canonical example scenes live in the `export_scenes` example, which dumps
//! each as JSON over defmt; save one under `scenes/` to try `/load`.
//!
//! Run with (bare ESP32, the default): `cargo run --release`
//!
//! Run with (Alvik):
//! `cargo run --release --no-default-features --features alvik,router,rhai`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;

#[beet_esp::main]
fn main() {
    let mut app = App::new();
    app.add_plugins((
        Esp32Plugin,
        HealthPlugin,
        // The static IPv4 comes from the `WIFI_STATIC_IP` env var, so a
        // controller reaches the device at a fixed address with no lookup.
        WifiPlugin::from_env().with_env_static_ip(),
        RouterPlugin,
        EspScenePlugin,
    ))
    .add_systems(Startup, setup);

    // The Alvik build drives the robot; the bare build drives the on-board
    // WS2812. Both expose `LedColor` entities, so the generic scene routes and
    // the reset handler work either way.
    cfg_if! {
      if #[cfg(feature = "alvik")]{
        app.add_plugins(AlvikPlugin);
      }else{
        app.add_plugins(LedPlugin)
          .add_systems(Startup, setup_builtin_led);
      }
    }

    app.run();
}

fn setup_builtin_led(mut commands: Commands) {
    commands.spawn((LedColor::default(), Ws2812Led));
}

// The bootstrap server: only the meta-routes. The real routes arrive via
// `/load`. `BeetSceneRoot`s get reparented here and picked up by the router.
// The router's default not-found middleware serves a route listing at `/`.
fn setup(mut commands: Commands) {
    commands.spawn((
        HttpServer::new(DEFAULT_SERVER_PORT),
        default_router(),
        children![
            exchange_route("load", LoadScene),
            exchange_route("clear", ClearScene),
            exchange_route("reset", Reset),
            exchange_route("dump", DumpScene),
        ],
    ));
}
