//! The scene server: a bootstrap HTTP API whose *real* routes arrive over the
//! wire as a beet scene. The meta-routes here ([`LoadScene`], [`ClearScene`],
//! [`Reset`], [`DumpScene`], [`Home`]) load, swap and inspect that scene; the
//! behaviours it wires are the [`routes`](super::routes) and
//! [`actions`](super::actions) markers. Add [`AlvikScenePlugin`] to register
//! every type a scene can carry.

use crate::prelude::*;
use beet::prelude::*;
use defmt::Debug2Format;
use defmt::info;
use defmt::warn;

extern crate alloc;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// Registers every route/action/scene marker a loaded scene can carry, so
/// reflection can (de)serialize it. The router, `Sequence`/`Repeat` and
/// `RunTimer` types are covered by [`RouterPlugin`]/[`ActionPlugin`].
pub struct AlvikScenePlugin;

impl Plugin for AlvikScenePlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<DriveRoute>()
            .register_type::<LedRoute>()
            .register_type::<ActionRoute>()
            .register_type::<ApplyDrive>()
            .register_type::<DriveCommand>()
            .register_type::<LineFollowStep>()
            .register_type::<RoombaStep>()
            .register_type::<EndInDuration>();
        #[cfg(feature = "scripting")]
        app.register_type::<super::scripting::AlvikScript>()
            .register_type::<super::scripting::AlvikScriptStep>();
    }
}

/// Marker for a root entity spawned from a loaded scene. Reparented under the
/// server so the router picks it up; despawned wholesale on the next `/load`
/// or on `/clear`.
#[derive(Component)]
pub struct SceneRoot;

/// `POST /load` — load a scene from the request body (JSON or postcard, per the
/// `content-type`), replacing any previously loaded scene.
///
/// Under one exclusive world lock so no frame runs mid-swap: despawn the old
/// [`SceneRoot`]s, reset the robot, deserialize the body, reparent the new roots
/// under the server, and rebuild the [`RouteTree`].
#[action(handler_only)]
#[derive(Default, Clone, Component)]
pub async fn LoadScene(cx: ActionContext<Request>) -> Response {
    let media = match cx.input.into_media_bytes().await {
        Ok(media) => media,
        Err(err) => {
            warn!("scene: bad request body: {}", Debug2Format(&err));
            return Response::status_text(
                StatusCode::BAD_REQUEST,
                format!("failed to read scene body: {err}\n"),
            );
        }
    };

    cx.caller
        .with_world(move |world, caller| -> Response {
            world.run_system_cached(despawn_scene_roots).ok();
            world.run_system_cached(reset_robot).ok();

            let spawned = match WorldSerdeLoader::new(world).load(&media) {
                Ok(spawned) => spawned,
                Err(err) => {
                    warn!("scene: invalid scene: {}", Debug2Format(&err));
                    return Response::status_text(
                        StatusCode::BAD_REQUEST,
                        format!("invalid scene: {err}\n"),
                    );
                }
            };

            // roots are the spawned entities without a parent; reparent them
            // under the server so the router sees them as routes.
            let server = world.root_ancestor(caller);
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
            if let Ok(Err(err)) =
                world.run_system_cached_with(RouteTree::rebuild, server)
            {
                warn!("scene: route tree rebuild failed: {}", Debug2Format(&err));
                return Response::status_text(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "route tree rebuild failed\n",
                );
            }

            info!("scene: loaded {} root route(s)", roots.len());
            Response::ok_text(format!("loaded scene: {} root(s)\n", roots.len()))
        })
        .await
        .unwrap_or_else(|err| {
            warn!("scene: load failed: {}", Debug2Format(&err));
            Response::status_text(StatusCode::INTERNAL_SERVER_ERROR, "scene load failed\n")
        })
}

/// `GET /clear` — despawn the loaded scene and reset the robot.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
pub async fn ClearScene(cx: ActionContext<RequestParts>) -> Response {
    cx.caller
        .with_world(|world, caller| {
            world.run_system_cached(despawn_scene_roots).ok();
            world.run_system_cached(reset_robot).ok();
            // rebuild the tree so the cleared routes drop out of dispatch.
            let server = world.root_ancestor(caller);
            if let Ok(Err(err)) =
                world.run_system_cached_with(RouteTree::rebuild, server)
            {
                warn!("scene: route tree rebuild failed: {}", Debug2Format(&err));
            }
        })
        .await
        .ok();
    info!("scene: cleared");
    Response::ok_text("scene cleared\n")
}

/// `GET /reset` — stop the motors and turn the UI LEDs off, leaving any loaded
/// scene in place.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
pub async fn Reset(cx: ActionContext<RequestParts>) -> Response {
    cx.caller
        .with_world(|world, _caller| {
            world.run_system_cached(reset_robot).ok();
        })
        .await
        .ok();
    info!("scene: reset");
    Response::ok_text("reset\n")
}

/// `GET /dump` — serialize the currently loaded scene (the [`SceneRoot`] trees)
/// back to JSON. Empty when no scene is loaded.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
pub async fn DumpScene(cx: ActionContext<RequestParts>) -> Response {
    cx.caller
        .with_world(|world, _caller| -> Response {
            let roots = world
                .query_filtered::<Entity, With<SceneRoot>>()
                .iter(world)
                .collect::<Vec<_>>();
            let mut saver = WorldSerdeSaver::new(world);
            for root in roots {
                saver = saver.with_entity_tree(root);
            }
            match saver
                .save(MediaType::Json)
                .and_then(|bytes| bytes.as_utf8().map(String::from))
            {
                Ok(json) => Response::ok_body(json, MediaType::Json),
                Err(err) => {
                    warn!("scene: dump failed: {}", Debug2Format(&err));
                    Response::status_text(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("dump failed: {err}\n"),
                    )
                }
            }
        })
        .await
        .unwrap_or_else(|err| {
            warn!("scene: dump failed: {}", Debug2Format(&err));
            Response::status_text(StatusCode::INTERNAL_SERVER_ERROR, "dump failed\n")
        })
}

/// `GET /` — list the meta-routes and the currently loaded scene routes.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
pub async fn Home(cx: ActionContext<RequestParts>) -> Response {
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
    Response::ok_text(format!(
        "alvik scene server\n\nmeta routes:\n  /load   POST a scene (json|postcard)\n  /clear  despawn scene + reset\n  /reset  stop motors, leds off\n  /dump   current scene as json\n\nactive routes:\n{routes}"
    ))
}

/// Despawn every [`SceneRoot`] (and its descendants).
pub fn despawn_scene_roots(roots: Query<Entity, With<SceneRoot>>, mut commands: Commands) {
    for root in &roots {
        commands.entity(root).despawn();
    }
}

/// Stop the motors and turn the UI LEDs off — the safe resting state.
pub fn reset_robot(
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
// Example scenes, built in code and dumped as JSON on boot.
// ---------------------------------------------------------------------------

/// `dance-routine` — forward 1s, left 1s, forward 1s, stop.
pub fn dance_scene() -> impl Bundle {
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
pub fn line_follower_scene() -> impl Bundle {
    (ActionRoute, PathPartial::new("line-follower"), children![(
        Repeat::new(),
        children![(
            Sequence::new(),
            children![LineFollowStep, EndInDuration::pass(Duration::from_millis(50))],
        )],
    )])
}

/// `roomba` — repeat a wall-avoiding step every 50 ms.
pub fn roomba_scene() -> impl Bundle {
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
pub fn log_example_scenes(world: &mut World) {
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
    dump_scene(world, "script", super::scripting::script_scene());
}
