//! Reflectable route markers: the one-shot behaviours a scene can wire to an
//! HTTP path. Each is an `#[action(route)]` component, so a loaded scene binds
//! it to a path and the firmware dispatches matching requests to it.

use crate::prelude::*;
use beet::prelude::*;
use defmt::info;

extern crate alloc;
use alloc::format;

/// Forward/back speed for [`DriveRoute`], mm/s.
const DRIVE_SPEED_MM_S: f32 = 60.0;
/// Turn rate (spin in place) for [`DriveRoute`], deg/s.
const TURN_RATE_DEG_S: f32 = 90.0;

/// `:dir` -> a continuous drive velocity. A scene binds this to a path, eg
/// `drive/:dir`, and the robot keeps moving until `stop` (true RC semantics).
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub fn DriveRoute(
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
    Response::ok_text(format!("drive {dir}\n"))
}

/// `:side`/`:state` -> one UI LED white (on) or black (off).
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub fn LedRoute(
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
    Response::ok_text(format!("led {side} {state}\n"))
}

/// Wires an HTTP path to a behaviour tree. The tree is the route entity's
/// single child; calling the route spawns a detached task that runs it, then
/// returns at once. A scene supplies the path and the tree under it.
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
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
