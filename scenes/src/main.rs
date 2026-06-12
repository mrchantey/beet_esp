//! Host scene generator: build each canonical beet_esp scene and serialize it to
//! `../target/scenes/<name>.json`.
//!
//! The scene-definition types live in `beet_esp` (its ECS components,
//! `#[action]`s and scene bundles), which compiles for the host without its
//! `device` hardware stack. It is a regular [`CliServer`], built the same way as
//! the upstream `beet-cli` `export_scenes` example: an [`ExportScenes`] root
//! route whose children are the scene roots, each carrying its [`ExportPath`].
//! Running with no args writes them all to `target/` (gitignored) on demand.
//!
//! Run with: `cd scenes && cargo run` (or `just export-scenes`).

use beet::prelude::*;
use beet_esp::prelude::*;

fn main() -> AppExit {
    // RouterPlugin + ServerPlugin drive the CliServer (route dispatch + async);
    // EspScenePlugin registers every scene type so reflection can serialize them.
    // No hardware plugins (Esp32Plugin/HealthPlugin) — they are device-only and
    // the scenes need only the reflect registrations. MinimalPlugins + LogPlugin
    // give the schedule a runner and log output.
    App::new()
        .add_plugins((
            MinimalPlugins,
            LogPlugin::default(),
            RouterPlugin,
            ServerPlugin,
            EspScenePlugin,
        ))
        .add_systems(Startup, spawn_host)
        .run()
}

/// `<scenes-crate>/../target/scenes/<label>.json` — the firmware crate's
/// `target/` is one directory up. Built from `CARGO_MANIFEST_DIR` so it is
/// independent of the working directory `cargo run` is invoked from.
fn scene_path(label: &str) -> String {
    format!("{}/../target/scenes/{label}.json", env!("CARGO_MANIFEST_DIR"))
}

/// Spawn the export host: a [`CliServer`] router whose root [`ExportScenes`]
/// route writes each of its children as a standalone scene. The canonical
/// beet_esp scenes are declared as that route's children, each carrying its
/// [`ExportPath`]; running with no args serializes them all to disk.
fn spawn_host(mut commands: Commands) {
    commands.spawn((CliServer, default_router(), children![(
        ExportScenes,
        children![
            (ExportPath(scene_path("led-script")), led_script_scene()),
            // rc: two standalone route roots grouped under one `Router` scene root.
            (
                ExportPath(scene_path("rc")),
                Router,
                children![
                    (DriveRoute, PathPartial::new("drive/:dir")),
                    (LedRoute, PathPartial::new("led/:side/:state")),
                ],
            ),
            (ExportPath(scene_path("dance-routine")), dance_scene()),
            (ExportPath(scene_path("line-follower")), line_follower_scene()),
            (ExportPath(scene_path("roomba")), roomba_scene()),
            (ExportPath(scene_path("script")), script_scene()),
        ],
    )]));
}

// ---------------------------------------------------------------------------
// The bare-ESP32 scene: the on-board WS2812 driven by a rhai script.
// ---------------------------------------------------------------------------

/// A demo LED script: cycle red/green/blue from the elapsed time, counting the
/// ticks it has run in `state` to show the persistent map at work.
const LED_SCRIPT: &str = r#"
let count = if "count" in input.state { input.state.count } else { 0 };
let phase = (input.elapsed_ms / 500) % 3;
let led = if phase == 0 { 0xff0000 } else if phase == 1 { 0x00ff00 } else { 0x0000ff };
#{
    led: led,
    state: #{ count: count + 1 },
}
"#;

/// `led-script` — repeat the demo LED [`Script`] every 100 ms.
fn led_script_scene() -> impl Bundle {
    (
        SpawnAction,
        PathPartial::new("led-script"),
        children![(
            Repeat::new(),
            children![(
                Sequence::new(),
                children![
                    (
                        LedScriptStep,
                        Script::<LedInput, LedOutput>::rhai(LED_SCRIPT),
                    ),
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
fn dance_scene() -> impl Bundle {
    (
        SpawnAction,
        PathPartial::new("dance-routine"),
        children![(
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
        )],
    )
}

/// `line-follower` — repeat a bang-bang follow step every 50 ms.
fn line_follower_scene() -> impl Bundle {
    (
        SpawnAction,
        PathPartial::new("line-follower"),
        children![(
            Repeat::new(),
            children![(
                Sequence::new(),
                children![
                    LineFollowStep,
                    EndInDuration::pass(Duration::from_millis(50))
                ],
            )],
        )],
    )
}

/// `roomba` — repeat a wall-avoiding step every 50 ms.
fn roomba_scene() -> impl Bundle {
    (
        SpawnAction,
        PathPartial::new("roomba"),
        children![(
            Repeat::new(),
            children![(
                Sequence::new(),
                children![RoombaStep, EndInDuration::pass(Duration::from_millis(50))],
            )],
        )],
    )
}

/// A demo Alvik script: back off when something is within 20 cm, otherwise
/// cruise, pulsing the LEDs from a counter held in `state`.
const ALVIK_SCRIPT: &str = r#"
let t = if "t" in input.state { input.state.t } else { 0 };
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
fn script_scene() -> impl Bundle {
    (
        SpawnAction,
        PathPartial::new("script"),
        children![(
            Repeat::new(),
            children![(
                Sequence::new(),
                children![
                    (
                        AlvikScriptStep,
                        Script::<AlvikInput, AlvikOutput>::rhai(ALVIK_SCRIPT),
                    ),
                    EndInDuration::pass(Duration::from_millis(100)),
                ],
            )],
        )],
    )
}
