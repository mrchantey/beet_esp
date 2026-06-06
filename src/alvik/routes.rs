//! Reflectable Alvik route markers: the one-shot behaviours a scene can wire to
//! an HTTP path. Each is an `#[action(route)]` component, so a loaded scene
//! binds it to a path and the firmware dispatches matching requests to it. The
//! hardware-agnostic [`ActionRoute`] lives upstream with the generic scene
//! server in [`beet::router`].

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
