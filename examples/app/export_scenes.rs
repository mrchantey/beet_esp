//! Export the canonical example scenes as JSON over defmt, so each can be saved
//! to `scenes/<name>.json` and POSTed to the firmware's `/load` (or loaded with
//! `beet load scenes/<name>.json`).
//!
//! The scene types live in this `no_std` crate, so the scenes are built on-chip
//! and streamed over RTT. Run on the device, then copy each logged JSON block
//! into `scenes/<name>.json`:
//!
//! ```sh
//! cargo run --release --example export_scenes              # bare ESP32: led-script
//! cargo run --release --no-default-features \
//!     --features alvik,router,rhai --example export_scenes # + alvik scenes
//! ```

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
// disambiguate our scene `Script` from beet's own `Script` action, both pulled in
// by the globs above.
use beet_esp::scripting::rhai::Script;
use defmt::Debug2Format;
use defmt::info;
use defmt::warn;

extern crate alloc;
use alloc::string::String;
use alloc::vec::Vec;

#[beet_esp::main]
fn main() {
    let mut app = App::new();
    // Esp32Plugin brings up the heap the serializer allocates in; RouterPlugin +
    // EspScenePlugin register every scene type so reflection can serialize them.
    app.add_plugins((Esp32Plugin, HealthPlugin, RouterPlugin, EspScenePlugin));
    app.add_systems(Startup, export_scenes);
    app.run();
}

/// Build each example scene, dump it as JSON over defmt, then despawn it.
#[allow(unused_variables)]
fn export_scenes(world: &mut World) {
    #[cfg(all(feature = "rhai", feature = "led"))]
    dump_scene(world, "led-script", led_script_scene());

    #[cfg(feature = "alvik")]
    {
        // rc: two standalone route roots in one scene.
        let drive = world.spawn((DriveRoute, PathPartial::new("drive/:dir"))).id();
        let led = world.spawn((LedRoute, PathPartial::new("led/:side/:state"))).id();
        dump_roots(world, "rc", [drive, led]);

        dump_scene(world, "dance-routine", dance_scene());
        dump_scene(world, "line-follower", line_follower_scene());
        dump_scene(world, "roomba", roomba_scene());
        #[cfg(feature = "rhai")]
        dump_scene(world, "script", script_scene());
    }
}

/// Spawn a one-root scene `bundle`, dump it, then despawn it.
fn dump_scene(world: &mut World, label: &str, bundle: impl Bundle) {
    let root = world.spawn(bundle).id();
    dump_roots(world, label, [root]);
}

/// Serialize the given scene `roots` to JSON via [`WorldSerdeSaver::save_roots`],
/// log it over defmt, then despawn them.
fn dump_roots(world: &mut World, label: &str, roots: impl IntoIterator<Item = Entity>) {
    let roots = roots.into_iter().collect::<Vec<_>>();
    let bytes = WorldSerdeSaver::save_roots(world, MediaType::Json, roots.iter().copied());
    roots.iter().for_each(|root| world.entity_mut(*root).despawn());
    match bytes.and_then(|bytes| bytes.as_utf8().map(String::from)) {
        Ok(json) => info!("scene[{}]:\n{}", label, json.as_str()),
        Err(err) => warn!("scene[{}] dump failed: {}", label, Debug2Format(&err)),
    }
}

// ---------------------------------------------------------------------------
// The bare-ESP32 scene: the on-board WS2812 driven by a rhai script.
// ---------------------------------------------------------------------------

/// A demo LED script: cycle red/green/blue from the elapsed time, counting the
/// ticks it has run in `state` to show the persistent map at work.
#[cfg(all(feature = "rhai", feature = "led"))]
const LED_SCRIPT: &str = r#"
let count = if "count" in state { state.count } else { 0 };
let phase = (input.elapsed_ms / 500) % 3;
let led = if phase == 0 { 0xff0000 } else if phase == 1 { 0x00ff00 } else { 0x0000ff };
#{
    led: led,
    state: #{ count: count + 1 },
}
"#;

/// `led-script` — repeat the demo LED [`Script`] every 100 ms.
#[cfg(all(feature = "rhai", feature = "led"))]
fn led_script_scene() -> impl Bundle {
    (
        SpawnAction,
        PathPartial::new("led-script"),
        children![(
            Repeat::new(),
            children![(
                Sequence::new(),
                children![
                    (LedScriptStep, Script {
                        source: String::from(LED_SCRIPT),
                        state: ScriptMap::default(),
                    }),
                    EndInDuration::pass(Duration::from_millis(100)),
                ],
            )],
        )],
    )
}

// ---------------------------------------------------------------------------
// The Alvik scenes.
// ---------------------------------------------------------------------------

/// `dance-routine` — forward 1s, left 1s, forward 1s, stop.
#[cfg(feature = "alvik")]
fn dance_scene() -> impl Bundle {
    (SpawnAction, PathPartial::new("dance-routine"), children![(
        Sequence::new(),
        children![
            (ApplyDrive, DriveCommand::drive(60.0, 0.0)),
            EndInDuration::pass(Duration::from_secs(1)),
            (ApplyDrive, DriveCommand::drive(0.0, 90.0)),
            EndInDuration::pass(Duration::from_secs(1)),
            (ApplyDrive, DriveCommand::drive(60.0, 0.0)),
            EndInDuration::pass(Duration::from_secs(1)),
            (ApplyDrive, DriveCommand::drive(0.0, 0.0)),
        ],
    )])
}

/// `line-follower` — repeat a bang-bang follow step every 50 ms.
#[cfg(feature = "alvik")]
fn line_follower_scene() -> impl Bundle {
    (SpawnAction, PathPartial::new("line-follower"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![LineFollowStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// `roomba` — repeat a wall-avoiding step every 50 ms.
#[cfg(feature = "alvik")]
fn roomba_scene() -> impl Bundle {
    (SpawnAction, PathPartial::new("roomba"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![RoombaStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// A demo Alvik script: back off when something is within 20 cm, otherwise
/// cruise, pulsing the LEDs from a counter held in `state`.
#[cfg(all(feature = "alvik", feature = "rhai"))]
const ALVIK_SCRIPT: &str = r#"
let t = if "t" in state { state.t } else { 0 };
let near = input.depth_mm > 0 && input.depth_mm < 200;
let bright = if t % 6 < 3 { 255 } else { 20 };
#{
    linear: if near { -40.0 } else { 50.0 },
    angular: if near { 90.0 } else { 0.0 },
    led_left: bright * 256,
    led_right: bright,
    state: #{ t: t + 1 },
}
"#;

/// `script` — repeat the demo Alvik [`Script`] every 100 ms.
#[cfg(all(feature = "alvik", feature = "rhai"))]
fn script_scene() -> impl Bundle {
    (SpawnAction, PathPartial::new("script"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![
                (AlvikScriptStep, Script {
                    source: String::from(ALVIK_SCRIPT),
                    state: ScriptMap::default(),
                }),
                EndInDuration::pass(Duration::from_millis(100)),
            ],
        )],
    )])
}
