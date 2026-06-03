//! Reflectable route markers a scene can wire to an HTTP path: the generic ones
//! that need no particular hardware. Domain-specific routes (the Alvik's
//! `DriveRoute`/`LedRoute`) live with their module.

use beet::prelude::*;
use defmt::info;

/// Wires an HTTP path to a behaviour tree. The tree is the route entity's
/// single child; calling the route spawns a detached task that runs it, then
/// returns at once. A scene supplies the path and the tree under it.
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "scene"]
pub async fn ActionRoute(cx: ActionContext<RequestParts>) -> Response {
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
            Response::ok_text("action started\n")
        }
        None => Response::ok_text("no behaviour to run\n"),
    }
}
