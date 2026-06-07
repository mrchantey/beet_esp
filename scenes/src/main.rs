//! Host scene generator: build each canonical beet_esp scene and serialize it to
//! `target/scenes/<name>.json`.
//!
//! The scene-definition types live in `beet_esp` (its ECS components,
//! `#[action]`s and scene bundles), which compiles for the host without its
//! `device` hardware stack. So unlike the old on-device `export_scenes` example
//! (which dumped JSON over defmt and was copy-pasted into git), this program
//! constructs the scenes on the PC and writes the JSON files directly. They land
//! under `target/` (gitignored) and are regenerated on demand.
//!
//! Run with: `cd scenes && cargo run --release`

use beet::prelude::*;
use beet_esp::prelude::*;
// disambiguate our scene `Script` from beet's own `Script` action, both pulled in
// by the globs above.
use beet_esp::scripting::rhai::Script;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

// beet_esp always links `defmt` (its scene types `#[derive(defmt::Format)]` and
// call `defmt::info!`). On the device a real RTT logger provides these symbols;
// on the host nothing does, so we link a no-op one. The scene types we build
// never actually log here, so discarding is fine.
defmt::timestamp!("");

#[defmt::global_logger]
struct NoopLogger;

unsafe impl defmt::Logger for NoopLogger {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

fn main() {
    // EspScenePlugin + RouterPlugin register every scene type so reflection can
    // serialize them. No hardware plugins (Esp32Plugin/HealthPlugin) here — they
    // are device-only and the scenes need only the reflect registrations.
    let mut app = App::new();
    app.add_plugins((RouterPlugin, EspScenePlugin));

    let out_dir = scenes_dir();
    fs::create_dir_all(&out_dir).expect("failed to create target/scenes");

    let world = app.world_mut();
    export_scenes(world, &out_dir);

    println!("wrote scenes to {}", out_dir.display());
}

/// `<workspace>/target/scenes`. The generator lives in `beet_esp/scenes`, so the
/// firmware crate's `target/` is one directory up.
fn scenes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("scenes crate has a parent dir")
        .join("target")
        .join("scenes")
}

/// Build each example scene and write it to `<out_dir>/<name>.json`.
fn export_scenes(world: &mut World, out_dir: &Path) {
    save_scene(world, out_dir, "led-script", led_script_scene());

    // rc: two standalone route roots in one scene.
    let drive = world
        .spawn((DriveRoute, PathPartial::new("drive/:dir")))
        .id();
    let led = world
        .spawn((LedRoute, PathPartial::new("led/:side/:state")))
        .id();
    save_roots(world, out_dir, "rc", [drive, led]);

    save_scene(world, out_dir, "dance-routine", dance_scene());
    save_scene(world, out_dir, "line-follower", line_follower_scene());
    save_scene(world, out_dir, "roomba", roomba_scene());
    save_scene(world, out_dir, "script", script_scene());
}

/// Spawn a one-root scene `bundle`, write it, then despawn it.
fn save_scene(world: &mut World, out_dir: &Path, label: &str, bundle: impl Bundle) {
    let root = world.spawn(bundle).id();
    save_roots(world, out_dir, label, [root]);
}

/// Serialize the given scene `roots` to JSON via [`WorldSerdeSaver::save_roots`],
/// write it to `<out_dir>/<label>.json`, then despawn them.
fn save_roots(
    world: &mut World,
    out_dir: &Path,
    label: &str,
    roots: impl IntoIterator<Item = Entity>,
) {
    let roots = roots.into_iter().collect::<Vec<_>>();
    let bytes = WorldSerdeSaver::save_roots(world, MediaType::Json, roots.iter().copied())
        .expect("failed to serialize scene");
    let json = bytes.as_utf8().expect("scene JSON is not UTF-8");
    let path = out_dir.join(format!("{label}.json"));
    fs::write(&path, json).expect("failed to write scene file");
    roots
        .iter()
        .for_each(|root| world.entity_mut(*root).despawn());
    println!("scene[{label}] -> {}", path.display());
}

// ---------------------------------------------------------------------------
// The bare-ESP32 scene: the on-board WS2812 driven by a rhai script.
// ---------------------------------------------------------------------------

/// A demo LED script: cycle red/green/blue from the elapsed time, counting the
/// ticks it has run in `state` to show the persistent map at work.
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
                        Script {
                            source: String::from(LED_SCRIPT),
                            state: ScriptMap::default(),
                        }
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
                        Script {
                            source: String::from(ALVIK_SCRIPT),
                            state: ScriptMap::default(),
                        }
                    ),
                    EndInDuration::pass(Duration::from_millis(100)),
                ],
            )],
        )],
    )
}
