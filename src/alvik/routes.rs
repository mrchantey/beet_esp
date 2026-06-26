//! Reflectable Alvik route handlers and their path-binding templates: the
//! one-shot behaviours a scene can wire to an HTTP path. Each handler is an
//! `#[action(route)]` component, and its `<DriveRoute path="..."/>` template binds
//! it to a path (inserting the [`PathPartial`] the router dispatches on, the way
//! upstream's `ScriptRoute` does — so a scene names a path, not a parsed segment
//! list). The hardware-agnostic [`SpawnAction`] lives upstream with the generic
//! scene server in [`beet::router`].

use crate::prelude::*;
use beet::prelude::*;

extern crate alloc;
use alloc::format;
use alloc::string::String;

/// Forward/back speed for [`DriveHandler`], mm/s.
const DRIVE_SPEED_MM_S: f32 = 60.0;
/// Turn rate (spin in place) for [`DriveHandler`], deg/s.
const TURN_RATE_DEG_S: f32 = 90.0;

/// `<DriveRoute path="drive/:dir"/>` — bind the [`DriveHandler`] to a route path.
/// The template inserts the [`PathPartial`] the router dispatches on, alongside
/// the handler action.
#[template]
pub fn DriveRoute(#[prop(into)] path: String) -> impl Bundle {
    (PathPartial::new(path), DriveHandler)
}

/// `:dir` -> a continuous drive velocity. A scene binds this to a path, eg
/// `drive/:dir`, and the robot keeps moving until `stop` (true RC semantics).
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub fn DriveHandler(
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

/// `<LedRoute path="led/:side/:state"/>` — bind the [`LedHandler`] to a route path.
#[template]
pub fn LedRoute(#[prop(into)] path: String) -> impl Bundle {
    (PathPartial::new(path), LedHandler)
}

/// `:side`/`:state` -> one UI LED white (on) or black (off).
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "alvik"]
pub fn LedHandler(
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
