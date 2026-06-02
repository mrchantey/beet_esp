//! Scene-driven Alvik: the robot's API is *sent to it* as a beet scene, not
//! baked into the firmware.
//!
//! Where [`rc`](./rc.rs) hard-codes its `/drive` and `/led` routes, this example
//! ships a tiny bootstrap server with only three meta-routes — `load`, `clear`
//! and `reset` — and then *loads its real routes over the wire*. A scene
//! (a reflection-serialized slice of the ECS, see beet's `router_serde` example)
//! is POSTed to `/load`; the firmware deserializes it, marks the spawned roots
//! with [`SceneRoot`], reparents them under the server, and rebuilds the
//! [`RouteTree`]. From that moment the scene's routes *are* the API.
//!
//! The route behaviours (drive, led, ...) are still plain code — reflectable
//! [`#[action(route)]`](beet::prelude::action) components like [`DriveRoute`]
//! and [`LedRoute`]. What the scene supplies is the *wiring*: which component
//! lives at which path. So the same firmware can expose wildly different APIs
//! depending on the scene you send it.
//!
//! ## Meta-routes (always present)
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
//! On boot the firmware logs a canonical example scene over defmt (see
//! [`log_example_scene`]); save that to `scene.json` to try `/load`.
//!
//! Run with: `cargo run --release --no-default-features --features alvik,router --example alvik-scene`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::Debug2Format;
use defmt::info;
use defmt::warn;

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Static IPv4 the robot binds to (matches [`rc`](./rc.rs) so the same
/// controller reaches it).
const ALVIK_IP: [u8; 4] = [192, 168, 86, 222];

/// Forward/back speed for the drive route.
const DRIVE_SPEED_MM_S: f32 = 60.0;
/// Turn rate (spin in place) for the drive route.
const TURN_RATE_DEG_S: f32 = 90.0;

#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    let mut app = App::new();
    app.add_plugins((
        Esp32Plugin,
        HealthPlugin,
        WifiPlugin::from_env().with_static_ip(ALVIK_IP),
        AlvikPlugin,
        RouterPlugin,
    ))
    // The scene's route markers and behaviour-tree leaves must be registered
    // so reflection can (de)serialize them; the router, Sequence/Repeat and
    // RunTimer types are covered by RouterPlugin / ActionPlugin.
    .register_type::<DriveRoute>()
    .register_type::<LedRoute>()
    .register_type::<ActionRoute>()
    .register_type::<ApplyDrive>()
    .register_type::<DriveCommand>()
    .register_type::<LineFollowStep>()
    .register_type::<RoombaStep>()
    .register_type::<EndInDuration>()
    // Dump the canonical example scenes over defmt on boot.
    .add_systems(Startup, log_example_scenes);

    #[cfg(feature = "scripting")]
    app.register_type::<scripting::AlvikScript>()
        .register_type::<scripting::AlvikScriptStep>();

    // The bootstrap server: only the meta-routes. The real routes arrive via
    // `/load`. `SceneRoot`s get reparented here and picked up by the router.
    app.spawn((
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

// ---------------------------------------------------------------------------
// Scene plumbing
// ---------------------------------------------------------------------------

/// Marker for a root entity spawned from a loaded scene. Reparented under the
/// server so the router picks it up; despawned wholesale on the next `/load`
/// or on `/clear`.
#[derive(Component)]
struct SceneRoot;

/// `POST /load` — load a scene from the request body (JSON or postcard, per the
/// `content-type`), replacing any previously loaded scene.
///
/// Steps, all under one exclusive world lock so no frame runs mid-swap:
/// despawn the old [`SceneRoot`]s, reset the robot, deserialize the body,
/// reparent the new roots under the server, and rebuild the [`RouteTree`].
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn LoadScene(cx: ActionContext<Request>) -> Response {
    let caller = cx.id();
    let media = match cx.input.into_media_bytes().await {
        Ok(media) => media,
        Err(err) => {
            return Response::ok_body(
                format!("failed to read scene body: {err}\n"),
                MediaType::Text,
            );
        }
    };

    let result = cx
        .caller
        .world()
        .with(move |world: &mut World| -> Result<usize> {
            despawn_scene_roots(world);
            world.run_system_cached(reset_robot).ok();

            let spawned = WorldSerdeLoader::new(world).load(&media)?;

            // roots are the spawned entities without a parent; reparent them
            // under the server so the router sees them as routes.
            let server = root_ancestor(world, caller);
            let roots = spawned
                .iter()
                .copied()
                .filter(|entity| !world.entity(*entity).contains::<ChildOf>())
                .collect::<Vec<_>>();
            for root in roots.iter() {
                world.entity_mut(*root).insert((SceneRoot, ChildOf(server)));
            }

            // reparenting does not retrigger route-tree construction, so rebuild
            // it explicitly from the server's (now larger) descendant set.
            world.run_system_cached_with(rebuild_route_tree, server).ok();
            Ok(roots.len())
        })
        .await;

    match result {
        Ok(count) => {
            info!("scene: loaded {} root route(s)", count);
            Response::ok_body(format!("loaded scene: {count} root(s)\n"), MediaType::Text)
        }
        Err(err) => {
            warn!("scene: load failed: {}", Debug2Format(&err));
            Response::ok_body(format!("load failed: {err}\n"), MediaType::Text)
        }
    }
}

/// `GET /clear` — despawn the loaded scene and reset the robot.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn ClearScene(cx: ActionContext<RequestParts>) -> Response {
    let caller = cx.id();
    cx.caller
        .world()
        .with(move |world: &mut World| {
            despawn_scene_roots(world);
            world.run_system_cached(reset_robot).ok();
            // rebuild the tree so the cleared routes drop out of dispatch.
            let server = root_ancestor(world, caller);
            world.run_system_cached_with(rebuild_route_tree, server).ok();
        })
        .await;
    info!("scene: cleared");
    Response::ok_body("scene cleared\n", MediaType::Text)
}

/// `GET /reset` — stop the motors and turn the UI LEDs off, leaving any loaded
/// scene in place.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Reset(cx: ActionContext<RequestParts>) -> Response {
    cx.caller
        .world()
        .with(|world: &mut World| {
            world.run_system_cached(reset_robot).ok();
        })
        .await;
    info!("scene: reset");
    Response::ok_body("reset\n", MediaType::Text)
}

/// `GET /dump` — serialize the currently loaded scene (the [`SceneRoot`] trees)
/// back to JSON. Empty when no scene is loaded.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn DumpScene(cx: ActionContext<RequestParts>) -> Response {
    let json = cx
        .caller
        .world()
        .with(|world: &mut World| -> Result<String> {
            let roots = world
                .query_filtered::<Entity, With<SceneRoot>>()
                .iter(world)
                .collect::<Vec<_>>();
            let mut saver = WorldSerdeSaver::new(world);
            for root in roots {
                saver = saver.with_entity_tree(root);
            }
            saver.save(MediaType::Json)?.as_utf8().map(String::from)
        })
        .await;
    match json {
        Ok(json) => Response::ok_body(json, MediaType::Json),
        Err(err) => Response::ok_body(format!("dump failed: {err}\n"), MediaType::Text),
    }
}

/// `GET /` — list the meta-routes and the currently loaded scene routes.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Home(cx: ActionContext<RequestParts>) -> Response {
    let routes = cx
        .caller
        .with_state::<AncestorQuery<&RouteTree>, String>(move |entity, query| {
            query
                .get(entity)
                .map(|tree| {
                    tree.flatten()
                        .iter()
                        .map(|pattern| format!("  /{}\n", pattern.annotated_path()))
                        .collect::<String>()
                })
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
    Response::ok_body(
        format!(
            "alvik scene server\n\nmeta routes:\n  /load   POST a scene (json|postcard)\n  /clear  despawn scene + reset\n  /reset  stop motors, leds off\n  /dump   current scene as json\n\nactive routes:\n{routes}"
        ),
        MediaType::Text,
    )
}

/// Despawn every [`SceneRoot`] (and its descendants).
fn despawn_scene_roots(world: &mut World) {
    let roots = world
        .query_filtered::<Entity, With<SceneRoot>>()
        .iter(world)
        .collect::<Vec<_>>();
    for root in roots {
        world.entity_mut(root).despawn();
    }
}

/// Walk [`ChildOf`] to the topmost ancestor of `entity`.
fn root_ancestor(world: &World, mut entity: Entity) -> Entity {
    while let Some(child_of) = world.entity(entity).get::<ChildOf>() {
        entity = child_of.parent();
    }
    entity
}

/// Rebuild the [`RouteTree`] on `server` from all of its descendant actions.
///
/// Mirrors the router's `insert_route_tree`, but runs on demand: reparenting a
/// scene under the server does not retrigger that observer.
fn rebuild_route_tree(
    server: In<Entity>,
    children_query: Query<&Children>,
    actions: Query<ActionQueryItem, Without<RouteHidden>>,
    mut commands: Commands,
) {
    let nodes = children_query
        .iter_descendants_inclusive(*server)
        .filter_map(|entity| actions.get(entity).ok())
        .map(ActionNode::from_query)
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return;
    }
    match RouteTree::from_nodes(nodes) {
        Ok(tree) => {
            commands.entity(*server).insert(tree);
        }
        Err(err) => warn!("scene: route tree rebuild failed: {}", Debug2Format(&err)),
    }
}

/// Stop the motors and turn the UI LEDs off — the safe resting state.
fn reset_robot(
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
    mut wheels: Query<&mut WheelTarget>,
    mut leds: Query<&mut LedColor, With<AlvikLed>>,
) {
    drive.linear = LinearVelocity::from_mm_per_sec(0.0);
    drive.angular = AngularVelocity::from_deg_per_sec(0.0);
    for mut target in &mut wheels {
        *target = WheelTarget::Speed(AngularVelocity::from_rpm(0.0));
    }
    for mut color in &mut leds {
        color.0 = Color::BLACK;
    }
}

// ---------------------------------------------------------------------------
// Reflectable route markers — the behaviours a scene can wire to a path.
// ---------------------------------------------------------------------------

/// `:dir` -> a continuous drive velocity. A scene binds this to a path, eg
/// `drive/:dir`, and the robot keeps moving until `stop` (true RC semantics).
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
fn DriveRoute(
    cx: In<ActionContext<RequestParts>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Response {
    let dir = cx.input.get_param("dir").unwrap_or("stop");
    let (linear, angular) = match dir {
        "forward" => (DRIVE_SPEED_MM_S, 0.0),
        "back" => (-DRIVE_SPEED_MM_S, 0.0),
        "left" => (0.0, TURN_RATE_DEG_S),
        "right" => (0.0, -TURN_RATE_DEG_S),
        _ => (0.0, 0.0),
    };
    drive.linear = LinearVelocity::from_mm_per_sec(linear);
    drive.angular = AngularVelocity::from_deg_per_sec(angular);
    info!("scene: drive {} ({} mm/s, {} deg/s)", dir, linear, angular);
    Response::ok_body(format!("drive {dir}\n"), MediaType::Text)
}

/// `:side`/`:state` -> one UI LED white (on) or black (off).
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
fn LedRoute(
    cx: In<ActionContext<RequestParts>>,
    mut leds: Query<(&AlvikLed, &mut LedColor)>,
) -> Response {
    let side = cx.input.get_param("side").unwrap_or("");
    let state = cx.input.get_param("state").unwrap_or("off");
    let want = match side {
        "left" => Side::Left,
        _ => Side::Right,
    };
    let color = if state == "on" { Color::WHITE } else { Color::BLACK };
    for (_, mut led_color) in leds.iter_mut().filter(|(led, _)| led.side == want) {
        led_color.0 = color;
    }
    info!("scene: led {} {}", side, state);
    Response::ok_body(format!("led {side} {state}\n"), MediaType::Text)
}

// ---------------------------------------------------------------------------
// Step 2: action routes — a route whose behaviour is a *behaviour tree* that
// plays out over time, not a one-shot. The tree is part of the scene: hitting
// the route fires it on the async pool and returns immediately, so even an
// endless control loop (line follower, roomba) doesn't block the response.
// ---------------------------------------------------------------------------

/// Wires an HTTP path to a behaviour tree. The tree is the route entity's
/// single child; calling the route spawns a detached task that runs it, then
/// returns at once. A scene supplies the path and the tree under it.
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
async fn ActionRoute(cx: ActionContext<RequestParts>) -> Response {
    let caller = cx.caller.clone();
    let child = caller
        .get(|children: &Children| children.first().copied())
        .await
        .ok()
        .flatten();
    match child {
        Some(child) => {
            // fire-and-forget: drive the tree to completion on the local pool so
            // the HTTP response returns immediately even for endless loops.
            let world = caller.world();
            world
                .run_async_local(move |world: AsyncWorld| async move {
                    world.entity(child).call::<(), Outcome>(()).await?;
                    Result::Ok(())
                })
                .await;
            info!("scene: action route fired");
            Response::ok_body("action started\n", MediaType::Text)
        }
        None => Response::ok_body("no behaviour to run\n", MediaType::Text),
    }
}

/// A drive velocity a behaviour-tree leaf can apply. Carried alongside
/// [`ApplyDrive`] so a scene can configure each step's motion.
#[derive(Debug, Default, Clone, Component, Reflect)]
#[reflect(Component, Default)]
#[type_path = "alvik"]
struct DriveCommand {
    /// Forward speed, mm/s (negative = reverse).
    linear_mm_s: f32,
    /// Turn rate, deg/s (positive = left).
    angular_deg_s: f32,
}

impl DriveCommand {
    /// A command with the given linear (mm/s) and angular (deg/s) rates.
    const fn drive(linear_mm_s: f32, angular_deg_s: f32) -> Self {
        Self { linear_mm_s, angular_deg_s }
    }
}

/// Behaviour-tree leaf: apply this entity's [`DriveCommand`] to the robot, then
/// [`Outcome::PASS`]. Pair with [`EndInDuration`] in a [`Sequence`] to "drive
/// like this for N seconds".
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
#[require(DriveCommand)]
fn ApplyDrive(
    cx: In<ActionContext>,
    commands: Query<&DriveCommand>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Outcome {
    if let Ok(command) = commands.get(cx.id()) {
        drive.linear = LinearVelocity::from_mm_per_sec(command.linear_mm_s);
        drive.angular = AngularVelocity::from_deg_per_sec(command.angular_deg_s);
        info!(
            "scene: apply drive ({} mm/s, {} deg/s)",
            command.linear_mm_s, command.angular_deg_s
        );
    }
    Outcome::PASS
}

/// Line-sensor reading at or above this counts as "black" (over the line).
/// A rough guess for the Alvik's reflectance sensors — tune on the bench.
const LINE_BLACK_THRESHOLD: i16 = 500;

/// Behaviour-tree leaf: one bang-bang line-following step. Forward while the
/// left sensor sees white, steer right while it sees black. Loop it with
/// [`Repeat`] for a continuous follower.
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
fn LineFollowStep(
    _cx: In<ActionContext>,
    line: Single<&LineSensors, With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Outcome {
    if line.left >= LINE_BLACK_THRESHOLD {
        // left over the line — steer back to the right
        drive.linear = LinearVelocity::from_mm_per_sec(20.0);
        drive.angular = AngularVelocity::from_deg_per_sec(-60.0);
    } else {
        drive.linear = LinearVelocity::from_mm_per_sec(40.0);
        drive.angular = AngularVelocity::from_deg_per_sec(0.0);
    }
    Outcome::PASS
}

/// Turn away from a wall closer than this (mm).
const ROOMBA_NEAR_MM: f32 = 200.0;

/// Behaviour-tree leaf: one roomba step. Drive forward until the centre ToF
/// reads a wall closer than [`ROOMBA_NEAR_MM`], then spin right to clear it.
/// Loop it with [`Repeat`].
#[action(handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
fn RoombaStep(
    _cx: In<ActionContext>,
    tof: Single<&Tof, With<AlvikRobot>>,
    mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
) -> Outcome {
    if tof.center.as_millimeters() <= ROOMBA_NEAR_MM {
        drive.linear = LinearVelocity::from_mm_per_sec(0.0);
        drive.angular = AngularVelocity::from_deg_per_sec(-90.0);
    } else {
        drive.linear = LinearVelocity::from_mm_per_sec(50.0);
        drive.angular = AngularVelocity::from_deg_per_sec(0.0);
    }
    Outcome::PASS
}

// ---------------------------------------------------------------------------
// Example scenes, built in code and dumped as JSON on boot.
// ---------------------------------------------------------------------------

/// `dance-routine` — forward 1s, left 1s, forward 1s, stop.
fn dance_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("dance-routine"), children![(
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
fn line_follower_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("line-follower"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![LineFollowStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// `roomba` — repeat a wall-avoiding step every 50 ms.
fn roomba_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("roomba"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![RoombaStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// Serialize a one-root scene `bundle` to JSON and log it over defmt.
fn dump_scene(world: &mut World, label: &str, bundle: impl Bundle) {
    let root = world.spawn(bundle).id();
    let bytes = WorldSerdeSaver::new(world)
        .with_entity_tree(root)
        .save(MediaType::Json);
    world.entity_mut(root).despawn();
    match bytes.and_then(|bytes| bytes.as_utf8().map(String::from)) {
        Ok(json) => info!("scene[{}]:\n{}", label, json.as_str()),
        Err(err) => warn!("scene[{}] dump failed: {}", label, Debug2Format(&err)),
    }
}

/// Dump every example scene as JSON on boot, so each can be saved and POSTed to
/// `/load`. The RC scene has two roots, so it is serialized as a pair.
fn log_example_scenes(world: &mut World) {
    // rc: two standalone route roots in one scene.
    let drive = world.spawn((DriveRoute, PathPartial::new("drive/:dir"))).id();
    let led = world.spawn((LedRoute, PathPartial::new("led/:side/:state"))).id();
    let bytes = WorldSerdeSaver::new(world)
        .with_entity_tree(drive)
        .with_entity_tree(led)
        .save(MediaType::Json);
    world.entity_mut(drive).despawn();
    world.entity_mut(led).despawn();
    match bytes.and_then(|bytes| bytes.as_utf8().map(String::from)) {
        Ok(json) => info!("scene[rc]:\n{}", json.as_str()),
        Err(err) => warn!("scene[rc] dump failed: {}", Debug2Format(&err)),
    }

    dump_scene(world, "dance-routine", dance_scene());
    dump_scene(world, "line-follower", line_follower_scene());
    dump_scene(world, "roomba", roomba_scene());
    #[cfg(feature = "scripting")]
    dump_scene(world, "script", scripting::script_scene());
}

// ---------------------------------------------------------------------------
// Step 3: scripting. The route's behaviour is a *rhai script* carried in the
// scene. Each tick the step gathers a rich [`AlvikInput`] snapshot, runs the
// script, and applies the [`AlvikOutput`] it returns — including a `state`
// array the script owns as persistent memory. So a one-line program POSTed
// over the wire becomes the robot's controller.
//
// beet's own `Script` action is std-only (rhai pulls std there), so this wires
// rhai directly: it builds `no_std` (rhai 1.25 + libm + hashbrown), and the
// input/output are marshalled by hand through rhai `Map`s rather than serde
// (serde's `no_std` split fights rhai's). Enable with the `scripting` feature.
// ---------------------------------------------------------------------------

#[cfg(feature = "scripting")]
mod scripting {
    use super::*;
    use rhai::Dynamic;
    use rhai::Engine;
    use rhai::Map;
    use rhai::Scope;

    /// The sensor snapshot exposed to a script as the `input` map, plus the
    /// script-owned `state` array (its scratch memory between ticks).
    struct AlvikInput {
        elapsed_ms: i64,
        depth_mm: i64,
        line_left: i64,
        line_center: i64,
        line_right: i64,
        yaw_deg: f64,
        touch: i64,
    }

    /// What a script returns: drive velocity, both UI LED colours (packed
    /// `0xRRGGBB`), and the next `state` array.
    struct AlvikOutput {
        linear_mm_s: f32,
        angular_deg_s: f32,
        led_left: u32,
        led_right: u32,
        state: Vec<i64>,
    }

    /// A rhai control script carried in a scene. `source` is the program;
    /// `state` is the persistent memory it reads and rewrites each tick.
    #[derive(Debug, Default, Clone, Component, Reflect)]
    #[reflect(Component)]
    #[type_path = "alvik"]
    pub struct AlvikScript {
        /// rhai source. Reads `input.*` and `state`, returns a map.
        source: String,
        /// Script-owned scratch memory, threaded tick to tick.
        state: Vec<i64>,
    }

    /// Behaviour-tree leaf: gather input, run this entity's [`AlvikScript`],
    /// apply the output. Loop it with [`Repeat`] for a live controller.
    #[action(handler_only)]
    #[derive(Default, Clone, Component, Reflect)]
    #[reflect(Component)]
    #[type_path = "alvik"]
    #[require(AlvikScript)]
    pub fn AlvikScriptStep(
        cx: In<ActionContext>,
        time: Res<Time>,
        mut scripts: Query<&mut AlvikScript>,
        sensors: Single<(&Tof, &LineSensors, &Orientation, &TouchValue), With<AlvikRobot>>,
        mut drive: Single<&mut DifferentialDrive, With<AlvikRobot>>,
        mut leds: Query<(&AlvikLed, &mut LedColor)>,
    ) -> Outcome {
        let Ok(mut script) = scripts.get_mut(cx.id()) else {
            return Outcome::PASS;
        };
        let (tof, line, orientation, touch) = *sensors;
        let input = AlvikInput {
            elapsed_ms: (time.elapsed_secs_f64() * 1000.0) as i64,
            depth_mm: tof.center.as_millimeters() as i64,
            line_left: line.left as i64,
            line_center: line.center as i64,
            line_right: line.right as i64,
            yaw_deg: orientation.0.to_euler(EulerRot::XYZ).2 as f64 * 180.0
                / core::f64::consts::PI,
            touch: touch.0 as i64,
        };

        match run_alvik_script(&script.source, &input, &script.state) {
            Ok(output) => {
                drive.linear = LinearVelocity::from_mm_per_sec(output.linear_mm_s);
                drive.angular = AngularVelocity::from_deg_per_sec(output.angular_deg_s);
                for (led, mut color) in &mut leds {
                    let packed = match led.side {
                        Side::Left => output.led_left,
                        Side::Right => output.led_right,
                    };
                    color.0 = unpack_color(packed);
                }
                info!(
                    "scene: script -> ({} mm/s, {} deg/s) led {:#08x}/{:#08x}",
                    output.linear_mm_s, output.angular_deg_s, output.led_left, output.led_right
                );
                script.state = output.state;
            }
            Err(err) => warn!("scene: script error: {}", err.as_str()),
        }
        Outcome::PASS
    }

    /// Run `source` against the gathered `input` + `state`, returning the
    /// parsed [`AlvikOutput`]. Marshals through rhai [`Map`]s (no serde).
    fn run_alvik_script(
        source: &str,
        input: &AlvikInput,
        state: &[i64],
    ) -> core::result::Result<AlvikOutput, String> {
        let engine = Engine::new();
        let mut scope = Scope::new();

        let mut map = Map::new();
        map.insert("elapsed_ms".into(), Dynamic::from_int(input.elapsed_ms));
        map.insert("depth_mm".into(), Dynamic::from_int(input.depth_mm));
        map.insert("line_left".into(), Dynamic::from_int(input.line_left));
        map.insert("line_center".into(), Dynamic::from_int(input.line_center));
        map.insert("line_right".into(), Dynamic::from_int(input.line_right));
        map.insert("yaw_deg".into(), Dynamic::from_float(input.yaw_deg));
        map.insert("touch".into(), Dynamic::from_int(input.touch));
        scope.push("input", map);
        scope.push(
            "state",
            state.iter().copied().map(Dynamic::from_int).collect::<rhai::Array>(),
        );

        let result = engine
            .eval_with_scope::<Dynamic>(&mut scope, source)
            .map_err(|err| alloc::format!("{err}"))?;
        let out = result
            .try_cast::<Map>()
            .ok_or_else(|| String::from("script must return a map"))?;

        let float = |key: &str| {
            out.get(key)
                .and_then(|value| value.as_float().ok().or_else(|| value.as_int().ok().map(|int| int as f64)))
                .unwrap_or(0.0)
        };
        let int = |key: &str| out.get(key).and_then(|value| value.as_int().ok()).unwrap_or(0);
        let next_state = out
            .get("state")
            .and_then(|value| value.clone().try_cast::<rhai::Array>())
            .map(|array| array.iter().filter_map(|value| value.as_int().ok()).collect())
            .unwrap_or_default();

        Ok(AlvikOutput {
            linear_mm_s: float("linear") as f32,
            angular_deg_s: float("angular") as f32,
            led_left: int("led_left") as u32,
            led_right: int("led_right") as u32,
            state: next_state,
        })
    }

    /// Unpack a `0xRRGGBB` integer into a [`Color`].
    fn unpack_color(packed: u32) -> Color {
        Color::srgb_u8(
            (packed >> 16) as u8,
            (packed >> 8) as u8,
            packed as u8,
        )
    }

    /// A demo script: back off when something is within 20 cm, otherwise cruise,
    /// pulsing the LEDs from a counter held in `state`.
    const DEMO_SCRIPT: &str = r#"
let t = if state.len() > 0 { state[0] } else { 0 };
let near = input.depth_mm > 0 && input.depth_mm < 200;
let bright = if t % 6 < 3 { 255 } else { 20 };
#{
    linear: if near { -40.0 } else { 50.0 },
    angular: if near { 90.0 } else { 0.0 },
    led_left: bright * 256,
    led_right: bright,
    state: [t + 1],
}
"#;

    /// `script` — repeat the demo [`AlvikScript`] every 100 ms.
    pub fn script_scene() -> impl Bundle {
        (ActionRoute, PathPartial::new("script"), children![(
            Repeat::new(),
            children![(
                Sequence::new(),
                children![
                    (
                        AlvikScriptStep,
                        AlvikScript {
                            source: String::from(DEMO_SCRIPT),
                            state: Vec::new(),
                        },
                    ),
                    EndInDuration::pass(Duration::from_millis(100)),
                ],
            )],
        )])
    }
}
