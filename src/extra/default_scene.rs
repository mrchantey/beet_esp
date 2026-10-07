//! Boot-time default scene: the scene that travels with the firmware.
//!
//! Embeds a `.bsx` scene at build time (the `BEET_DEFAULT_SCENE` build env, default
//! `templates/alvik/perceive-act-body.bsx`) and loads it once on boot through the same
//! [`BeetSceneRoot::load`] path a pushed scene uses, so the device is ready the moment
//! it powers on with no host `beet load`. A later `beet load` replaces it (the load
//! despawns the prior [`BeetSceneRoot`] first) and `beet clear` clears it for good, so
//! the default is a convenience, not a lock-in: a dead controller or a new host at a
//! different address is still handled by pushing an updated scene, no reflash.

use beet::prelude::*;

/// The `.bsx` scene embedded at build time, loaded once on boot. Its path is the
/// `BEET_DEFAULT_SCENE` build env (default `templates/alvik/perceive-act-body.bsx`),
/// resolved to an absolute path by `build.rs` and exposed as `BEET_DEFAULT_SCENE_FILE`.
const DEFAULT_SCENE: &str = include_str!(env!("BEET_DEFAULT_SCENE_FILE"));

/// Loads the firmware's embedded [`DEFAULT_SCENE`] once the scene server is up, so a
/// freshly powered device serves its routes (and dials the agent) with no host
/// `beet load`.
pub struct DefaultScenePlugin;

impl Plugin for DefaultScenePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, load_default_scene);
    }
}

/// Set once the embedded default scene has been loaded (or its load attempted), so it
/// loads exactly once per boot and never fights a later `beet clear`/`load`.
#[derive(Resource)]
struct DefaultSceneLoaded;

/// Load [`DEFAULT_SCENE`] under the scene server's router once its [`RouteTree`]
/// exists (it has finished booting), marking it [`BeetSceneRoot`] so a pushed scene
/// cleanly replaces it. Runs each frame until it fires, then inserts
/// [`DefaultSceneLoaded`] so it never repeats.
fn load_default_scene(world: &mut World) {
    if world.contains_resource::<DefaultSceneLoaded>() {
        return;
    }
    // the router a server dispatches through, once its route tree is built. Until
    // then the server is still booting, so wait and retry next frame.
    let router = world.with_state::<(
        Query<(Entity, &ChildOf), (With<Router>, With<RouteTree>)>,
        Query<(), With<HttpServer>>,
    ), _>(|(routers, servers)| {
        routers
            .iter()
            .find(|(_, parent)| servers.contains(parent.parent()))
            .map(|(router, _)| router)
    });
    let Some(router) = router else {
        return;
    };
    // same path a `beet load` takes: despawn any prior scene, spawn this one under the
    // router, rebuild the route tree. The router's root ancestor is the robot, so the
    // body resolves it by root-ancestor fallback, exactly like a pushed scene.
    match BeetSceneRoot::load(world, &MediaBytes::new_bsx(DEFAULT_SCENE), Some(router)) {
        Ok(roots) => info!("loaded default scene: {} root(s)", roots.len()),
        Err(err) => error!("failed to load default scene: {err}"),
    }
    world.insert_resource(DefaultSceneLoaded);
}
