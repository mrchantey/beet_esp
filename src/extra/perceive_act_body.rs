//! The perceive-act **body**: the reflectable types a loaded scene wires to turn
//! the Alvik into the v3 socket body client.
//!
//! Mirrors the workspace `<MockBody>`/`<WgpuBody>`, but as generalizable firmware:
//! instead of a hardcoded socket root, [`PerceiveActBodyPlugin`] *registers* the
//! reflectable pieces a pushed `.bsx` composes — the [`AgentSocket`] transport and
//! the [`WhoAmi`] / `drive` capability routes — so the scene, not the firmware,
//! decides the agent url and which capability sits at which path (see
//! `templates/alvik/perceive-act-body.bsx`):
//!
//! ```xml
//! <AgentSocket url="ws://192.168.86.220:8338">
//!     <Route path="whoami" {WhoAmi}/>
//!     <Route path="drive" {DriveForDurationAction}/>
//! </AgentSocket>
//! ```
//!
//! On load the [`AgentSocket`] root keeps a [`Socket`] connected back to the
//! agent's socket server (a [`PersistentSocket`], redialling with backoff for
//! the life of the scene), enables the duplex Request/Response exchange, and its
//! route tree serves the agent's `whoami`/`drive` requests. Losing the
//! connection triggers a [`ResetScene`] via [`ResetOnDisconnect`], so the robot
//! halts (motors, wheels, LEDs) the moment its agent disappears.
//! [`DriveForDurationAction`] writes the [`DifferentialDrive`] the agent chose (the
//! same command the wgpu fox drives off) onto the Alvik and stamps a [`DriveStep`];
//! [`expire_drive_step`] zeroes the drive after the command's own duration, so each
//! command is a bounded step (like the wgpu fox) and the robot halts between
//! commands. The reply carries a [`SettleTime`] telling the agent how long to wait
//! out the movement.

use crate::prelude::*;
use beet::prelude::*;
use beet::prelude::sockets::*;

extern crate alloc;
use alloc::string::String;

/// Registers the perceive-act **body** capability types a loaded scene can carry:
/// the [`AgentSocket`] outbound-transport template and the [`WhoAmi`] /
/// [`DriveForDurationAction`] route handlers, plus the [`expire_drive_step`] system
/// that bounds each drive command. Add it under the `alvik` + `sockets` build so a
/// pushed `.bsx` resolves `<AgentSocket>` and `<Route ... {WhoAmi}>` /
/// `<Route ... {DriveForDurationAction}>`.
///
/// The generic scene machinery (`Route`, `SpawnAction`, the socket exchange) plus
/// the shared [`DriveForDuration`] wire type are registered upstream by
/// [`EspScenePlugin`](crate::scene::EspScenePlugin) / [`RouterPlugin`] /
/// [`ActionPlugin`]; this adds only the body's own types.
pub struct PerceiveActBodyPlugin;

impl Plugin for PerceiveActBodyPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<WhoAmi>()
            .register_type::<DriveForDurationAction>()
            .register_type::<DriveStep>()
            // the reconnecting transport + its disconnect reaction, so a pushed
            // scene can also carry them directly (AgentSocket bakes them in).
            .register_type::<PersistentSocket>()
            .register_type::<ResetOnDisconnect>()
            // The outbound socket-client transport: wraps a non-reflect codec
            // `Arc`, so it cannot be a bsx literal — a template provides it,
            // expanding at build to leave no marker to re-fire on reload.
            .register_template::<AgentSocket>()
            // ends each drive command after its duration, so the robot steps then stops.
            .add_systems(Update, expire_drive_step);
    }
}

/// `<AgentSocket url="wss://host:port">` — the outbound socket-client transport
/// for the body, hosting the capability routes declared as its children.
///
/// Keeps a [`Socket`] connected to the agent ([`PersistentSocket`]: dial with
/// backoff, redial on close, for the life of the scene), enables the duplex
/// Request/Response [`ExchangeSocket`], and adds a [`default_router`] whose
/// route tree (built from the child `<Route>`s) serves the requests the agent
/// originates. [`ResetOnDisconnect`] rides along: a dropped connection triggers
/// [`ResetScene`], halting the robot instead of letting the last drive command
/// spin forever. [`ExchangeSocket::json`] wraps a non-reflect codec `Arc`, so
/// the bundle rides this template rather than a scene literal.
///
/// `url` defaults to the `BEET_SOCKET_SERVER` build env (the same default the
/// esp [`Socket`] transport falls back to), so a scene may omit it on a
/// firmware built with that env set. A bare `host:port` is prefixed as `ws://`;
/// a `wss://` url needs the `secure` build.
#[template]
pub fn AgentSocket(
    /// The agent's socket url, eg `wss://192.168.86.220:8338`; defaults to the
    /// `BEET_SOCKET_SERVER` build env.
    #[prop(into)]
    url: Option<String>,
) -> impl Bundle {
    let url = url
        .or_else(|| option_env!("BEET_SOCKET_SERVER").map(String::from))
        .unwrap_or_default()
        .xmap(|url| match url.contains("://") {
            true => url,
            false => alloc::format!("ws://{url}"),
        });
    rsx! {
        <span {(
            PersistentSocket::new(url),
            ResetOnDisconnect,
            ExchangeSocket::json(),
            default_router(),
        )}>
            <Slot/>
        </span>
    }
}

/// The `whoami` handshake answer: this client is the `body`. The agent strips the
/// quoting, so a plain-text `body` is read as the role token.
///
/// Bound with `<Route path="whoami" {WhoAmi}/>`.
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "perceive_act"]
async fn WhoAmi(_cx: ActionContext<RequestParts>) -> Response {
    Response::ok_text("body")
}

/// Serve `drive`: write the [`DifferentialDrive`] the agent chose onto the Alvik's
/// robot root and stamp a [`DriveStep`] deadline from the command's own duration;
/// [`expire_drive_step`] zeroes the drive when it expires, so each command is a
/// bounded step and the robot halts between commands. Both the velocity and the
/// duration come from the [`DriveForDuration`] the agent serialized onto the
/// request, so the model chooses them per command (no static tuning).
///
/// The reply carries a [`SettleTime`] (the command's duration), and the agent's
/// `respond-multi-modal` sleeps that long before finishing, so the next photo
/// waits for the robot to stop. The wait lives agent-side because this handler's
/// beet task has no async-timer waker (the countdown rides bevy [`Time`] in the
/// schedule), and holding the reply on a bridged poll starves the schedule loop.
///
/// Bound with `<Route path="drive" {DriveForDurationAction}/>`. The robot is found
/// by a global `With<AlvikRobot>` query, so the socket client need not be nested
/// under the robot.
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "perceive_act"]
async fn DriveForDurationAction(cx: ActionContext<DriveForDuration>) -> Response {
    let drive = cx.input.drive;
    let duration = cx.input.duration;
    info!(
        "drive: ({} mm/s, {} deg/s) for {:?}",
        drive.linear.as_mm_per_sec(),
        drive.angular.as_deg_per_sec(),
        duration
    );
    // write the commanded velocity onto the robot root and return its entity, skipping
    // cleanly if the Alvik is not up yet (the model trails several round-trips, so in
    // practice it is).
    let world = cx.world();
    let robot = world
        .with_state::<Query<(Entity, &mut DifferentialDrive), With<AlvikRobot>>, _>(
            move |mut drives| {
                let (entity, mut robot_drive) = drives.iter_mut().next()?;
                *robot_drive = drive;
                Some(entity)
            },
        )
        .await;
    let settle = match robot {
        Some(robot) => {
            let _ = world
                .entity(robot)
                .insert(DriveStep {
                    timer: Timer::new(duration, TimerMode::Once),
                })
                .await;
            SettleTime::new(duration)
        }
        // no robot up: nothing will move, so nothing to settle
        None => SettleTime::NONE,
    };
    Response::ok_json(&settle).unwrap_or_else(|_| Response::ok())
}

/// Marks the robot as mid-step; [`expire_drive_step`] counts it down and stops the
/// drive.
#[derive(Debug, Clone, Component, Reflect)]
#[reflect(Component)]
struct DriveStep {
    /// Counts down the drive command's duration.
    timer: Timer,
}

/// End each drive command once its [`DriveStep`] timer finishes, zeroing the
/// [`DifferentialDrive`] so each command is a bounded step and the robot halts between
/// commands (and after the loop's final one). Uses bevy [`Time`] (ticked by the
/// esp's `TimePlugin`), so it runs in the schedule with no async-timer waker
/// constraint.
fn expire_drive_step(
    time: Res<Time>,
    mut robots: Query<(Entity, &mut DriveStep, &mut DifferentialDrive)>,
    mut commands: Commands,
) {
    for (entity, mut step, mut drive) in &mut robots {
        if step.timer.tick(time.delta()).is_finished() {
            info!("drive step complete, halting");
            *drive = DifferentialDrive::default();
            commands.entity(entity).remove::<DriveStep>();
        }
    }
}

