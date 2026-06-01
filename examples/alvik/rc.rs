//! Remote-control Alvik: drive the robot and toggle its UI LEDs over HTTP.
//!
//! Combines the [`AlvikPlugin`] (robot over UART) with beet's no_std router
//! served over Wi-Fi (see [`ecs_router`](../../ecs_router.rs)). Each route is a
//! Bevy system that mutates the robot's command components — [`DifferentialDrive`]
//! for motion, [`LedColor`] on the [`AlvikLed`] children for the lights — and the
//! Alvik transport systems carry the change to the wire on the next frame.
//!
//! The station binds a **static IP** ([`ALVIK_IP`]) via
//! [`WifiPlugin::with_static_ip`], so a controller can reach it at a fixed
//! address with no lookup (see the `controller/` CLI). The drive routes set a
//! *continuous* velocity — the robot
//! keeps moving until `/drive/stop` (true RC semantics), so always stop it when
//! done.
//!
//! ## Routes
//!
//! ```sh
//! curl http://192.168.86.222:8080/                  # this help
//! curl http://192.168.86.222:8080/drive/forward     # forward / back / left / right / stop
//! curl http://192.168.86.222:8080/led/left/on       # side: left|right   state: on|off
//! ```
//!
//! Run with: `cargo run --release --no-default-features --features alvik,router --example alvik-rc`

#![no_std]
#![no_main]

use beet::prelude::*;
use beet_esp::prelude::*;
use defmt::info;

extern crate alloc;

/// Static IPv4 the robot binds to, so the controller always knows where it is.
/// Pick one on your LAN's subnet, outside the DHCP pool to avoid a clash.
const ALVIK_IP: [u8; 4] = [192, 168, 86, 222];

/// Forward/back speed for the drive routes.
const DRIVE_SPEED_MM_S: f32 = 60.0;
/// Turn rate (spin in place) for the left/right routes.
const TURN_RATE_DEG_S: f32 = 90.0;

// The World/registry/router/request bulk lives in PSRAM (see `beet_esp::mem`),
// so internal SRAM only holds the radio + DMA + stack: the 64 KiB reserve (+ the
// reclaimed region) is plenty, matching `ecs_router`.
#[beet_esp::main(internal_reserve_kb = 64)]
fn main() {
    App::new()
        .add_plugins((
            Esp32Plugin,
            HealthPlugin,
            WifiPlugin::from_env().with_static_ip(ALVIK_IP),
            AlvikPlugin,
            RouterPlugin,
        ))
        // beet-style: the `HttpServer` carries the router and the routes as
        // sibling children, each binding a path to an action. Spawning it starts
        // the accept loop once Wi-Fi is up (at the static IP set above).
        .spawn((
            HttpServer::new(8080),
            default_router(),
            children![
                exchange_route("", Home),
                exchange_route("drive/:dir", Drive),
                exchange_route("led/:side/:state", Led),
            ],
        ))
        .run();
}

/// `GET /` — list the control routes.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Home(_cx: ActionContext<RequestParts>) -> Response {
    Response::ok_body(
        "alvik rc\n\nroutes:\n  /drive/<forward|back|left|right|stop>\n  /led/<left|right>/<on|off>\n",
        MediaType::Text,
    )
}

/// `GET /drive/:dir` — set a continuous drive velocity from the direction
/// segment. The velocity holds until changed, so `forward` keeps the robot
/// moving until `stop`.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Drive(
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
    info!("rc: drive {} ({} mm/s, {} deg/s)", dir, linear, angular);
    Response::ok_body(alloc::format!("drive {dir}\n"), MediaType::Text)
}

/// `GET /led/:side/:state` — set one UI LED to white (on) or black (off).
#[action(handler_only)]
#[derive(Default, Clone, Component)]
fn Led(
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
    info!("rc: led {} {}", side, state);
    Response::ok_body(alloc::format!("led {side} {state}\n"), MediaType::Text)
}
