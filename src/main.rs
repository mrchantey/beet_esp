//! Scene-driven firmware: the device's API is *sent to it* as a beet scene, not
//! baked in. This is the default binary for the crate — `cargo run` flashes it.
//!
//! The firmware ships a tiny bootstrap server with only meta-routes — `load`,
//! `clear`, `reset`, `dump` — and loads its real routes over the wire. A scene
//! (a reflection-serialized slice of the ECS) is POSTed to `/load`; from that
//! moment the scene's routes *are* the API. The behaviours it wires
//! ([`SpawnAction`], the behaviour-tree leaves, [`Script`]s) live upstream in
//! [`beet::router`] and in [`beet_esp::scene`]; the scene supplies only which
//! behaviour sits at which path.
//!
//! This runs on a bare ESP32 breakout: the on-board WS2812 is driven by a
//! `Script` (the `led-script` scene reads the elapsed tick + current colour and
//! returns the next colour, keeping a counter in its persistent `state` map).
//! Built with `--features alvik` it also brings up the Alvik robot, adding its
//! drive/led/dance/line-follower/roomba/script scenes.
//!
//! ```sh
//! curl http://192.168.86.222:8337/                 # this help + current routes
//! curl http://192.168.86.222:8337/dump             # current scene as JSON
//! curl --data-binary @scenes/led-script.bsx \
//!      -H 'content-type: application/x-bsx' \
//!      http://192.168.86.222:8337/load             # load a scene
//! curl http://192.168.86.222:8337/clear            # despawn scene + reset
//! curl http://192.168.86.222:8337/reset            # stop hardware
//! ```
//!
//! `/load` accepts a `.bsx` scene (parsed by beet's BSX engine, see
//! [`beet_esp::scene`]'s authoring widgets) or a reflection-serialized JSON scene;
//! the [`TemplateLoader`] dispatches on the `content-type`.
//!
//! Or drive it from the host with the `beet` CLI: `beet load scenes/led-script.bsx`
//! (with `BEET_REMOTE_URL` set to the device). The canonical example scenes are
//! the hand-authored `.bsx` files in `scenes/`, pushed to the device as needed.
//!
//! Run with (bare ESP32, the default): `cargo run --release`
//!
//! Run with (Alvik): `cargo run --release --features alvik`

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

    // The perceive-act body: registers the `<AgentSocket>` transport + the
    // `whoami`/`apply-heading` capability routes a pushed `.bsx` composes into a
    // socket body client (see `templates/alvik/perceive-act-body.bsx`). Needs the
    // Alvik (the robot the heading drives) and the socket exchange (back to the agent).
    #[cfg(all(feature = "alvik", feature = "sockets"))]
    app.add_plugins(PerceiveActBodyPlugin);

    app.run();
}

// Only the bare-ESP32 (non-alvik) build wires the on-board WS2812; the alvik
// branch drives the robot's LEDs instead, so this is dead there.
#[cfg(not(feature = "alvik"))]
fn setup_builtin_led(mut commands: Commands) {
    commands.spawn((LedColor::default(), Ws2812Led));
}

// The bootstrap server: only the meta-routes. The real routes arrive via
// `/load`. `BeetSceneRoot`s get reparented under the server's root ancestor and
// picked up by the router. The router's default not-found middleware serves a
// route listing at `/`.
//
// `BootOnLoad` is the upstream boot verb: on the server's `LoadTemplate` it boots
// the transport (the accept loop). `WifiPlugin`'s `boot_added_servers` fires that
// `LoadTemplate` for a freshly-spawned server (bare metal has no app template-load
// pipeline to fire it), so spawning this bundle is enough to serve — it finds the
// `HttpServer` by `Added<HttpServer>` wherever it sits in the tree.
//
// Under `alvik` the server is *nested under the robot*: the boot tree is the
// `AlvikRobot` root (carrying `DifferentialDrive` + every sensor/state component,
// with the wheel/servo/LED hardware as children) and the scene server as one more
// child. So a loaded behaviour's `AgentQuery` resolves its agent to the robot by
// root-ancestor fallback (the loaded `<RouteAction>` is reparented under the
// server, whose root ancestor is the robot) — no `{Alvik}` marker needed. The
// `RouteTree` is built on that same root ancestor, so the nested router's dispatch
// (which walks ancestors for the tree) still resolves every route.
//
// Spawned imperatively rather than through the `<Alvik>` template: a Rust `rsx!`
// wraps every child of a capitalized tag in a `SlotChild`, but `Router` is a plain
// component with no `<Slot>`, so `rsx!{ <Router><SceneServer/></Router> }` would
// leave unconsumed slot content and fail the build at boot. The declarative
// `<Alvik>` element (in `alvik::scenes`) stays available for a `.bsx` scene, where
// the parser routes a component's children as real children.
//
// The bare (non-alvik) build has no robot, so the server stays a plain root.
#[cfg(feature = "alvik")]
fn setup(mut commands: Commands) {
    commands.spawn((
        AlvikRobot,
        // Grouped into sub-bundles: a flat tuple would exceed the 15-element
        // `Bundle` impl. State, then sensors.
        (
            Connected::default(),
            FirmwareVersion::default(),
            BehaviorCode::default(),
            BatterState::default(),
            Illuminator(true),
            BuiltinLed(false),
        ),
        (
            LineSensors::default(),
            ColorSensor::default(),
            Tof::default(),
            Imu::default(),
            Orientation::default(),
            RobotPose::default(),
            // Commanded velocity: the upstream `Drive` leaf writes this on the
            // robot (its agent, resolved as the root ancestor), and `flush_drive`
            // sends it to the wire.
            DifferentialDrive::default(),
            TouchValue::default(),
            MotionValue::default(),
        ),
        children![
            // --- the robot's hardware: wheels, servos, RGB UI LEDs ---
            (
                Wheel { side: Side::Left },
                WheelState::default(),
                WheelTarget::Speed(AngularVelocity::default()),
            ),
            (
                Wheel { side: Side::Right },
                WheelState::default(),
                WheelTarget::Speed(AngularVelocity::default()),
            ),
            (Servo {
                id: ServoId::A,
                position: Angle::from_degrees(90.0),
            }),
            (Servo {
                id: ServoId::B,
                position: Angle::from_degrees(90.0),
            }),
            (AlvikLed { side: Side::Left }, LedColor::default()),
            (AlvikLed { side: Side::Right }, LedColor::default()),
            // --- the scene server: a child of the robot root ---
            (
                HttpServer::new(DEFAULT_HTTP_PORT),
                BootOnLoad,
                default_router(),
                children![
                    exchange_route("load", LoadScene),
                    exchange_route("clear", ClearScene),
                    exchange_route("reset", Reset),
                    exchange_route("dump", DumpScene),
                ],
            ),
        ],
    ));
}

#[cfg(not(feature = "alvik"))]
fn setup(mut commands: Commands) {
    commands.spawn((
        HttpServer::new(DEFAULT_HTTP_PORT),
        BootOnLoad,
        default_router(),
        children![
            exchange_route("load", LoadScene),
            exchange_route("clear", ClearScene),
            exchange_route("reset", Reset),
            exchange_route("dump", DumpScene),
        ],
    ));
}
