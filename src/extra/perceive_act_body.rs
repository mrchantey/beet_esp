//! The perceive-act **body**: the reflectable types a loaded scene wires to turn
//! the Alvik into the v3 socket body client.
//!
//! Mirrors the workspace `<MockBody>`/`<WgpuBody>`, but as generalizable firmware:
//! instead of a hardcoded socket root, [`PerceiveActBodyPlugin`] *registers* the
//! reflectable pieces a pushed `.bsx` composes — the [`AgentSocket`] transport and
//! the [`WhoAmi`]/[`ApplyHeading`] capability routes — so the scene, not the
//! firmware, decides the agent url and which capability sits at which path (see
//! `templates/alvik/perceive-act-body.bsx`):
//!
//! ```xml
//! <AgentSocket url="ws://192.168.86.220:8338">
//!     <Route path="whoami" {WhoAmi}/>
//!     <Route path="apply-heading" {ApplyHeading}/>
//! </AgentSocket>
//! ```
//!
//! On load the [`AgentSocket`] root keeps a [`Socket`] connected back to the
//! agent's socket server (a [`PersistentSocket`], redialling with backoff for
//! the life of the scene), enables the duplex Request/Response exchange, and its
//! route tree serves the agent's `whoami`/`apply-heading` requests. Losing the
//! connection triggers a [`ResetScene`] via [`ResetOnDisconnect`], so the robot
//! halts (motors, wheels, LEDs) the moment its agent disappears. [`ApplyHeading`]
//! maps the chosen [`Heading`] onto the Alvik's [`DifferentialDrive`] (the same
//! command the wgpu fox drives off) and stamps a [`DriveStep`];
//! [`expire_drive_step`] zeroes the drive after the [`DriveStepConfig`] budget,
//! so a heading is a bounded step (like the wgpu fox) and the robot halts
//! between commands. The reply's `settle_secs` tells the agent how long to wait
//! out the movement.

use crate::prelude::*;
use beet::prelude::*;
use beet::prelude::sockets::*;

extern crate alloc;
use alloc::string::String;

/// Registers the perceive-act **body** capability types a loaded scene can carry:
/// the [`AgentSocket`] outbound-transport template and the [`WhoAmi`] /
/// [`ApplyHeading`] route handlers, plus the [`expire_drive_step`] system that
/// bounds each heading. Add it under the `alvik` + `sockets` build so a pushed
/// `.bsx` resolves `<AgentSocket>` and `<Route ... {WhoAmi}>` /
/// `<Route ... {ApplyHeading}>`.
///
/// The generic scene machinery (`Route`, `SpawnAction`, the socket exchange) is
/// registered upstream by [`EspScenePlugin`](crate::scene::EspScenePlugin) /
/// [`RouterPlugin`]; this adds only the body's own types.
pub struct PerceiveActBodyPlugin;

impl Plugin for PerceiveActBodyPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<WhoAmi>()
            .register_type::<ApplyHeading>()
            .register_type::<Heading>()
            .register_type::<ApplyHeadingInput>()
            .register_type::<DriveStep>()
            .register_type::<DriveStepConfig>()
            // the reconnecting transport + its disconnect reaction, so a pushed
            // scene can also carry them directly (AgentSocket bakes them in).
            .register_type::<PersistentSocket>()
            .register_type::<ResetOnDisconnect>()
            // The outbound socket-client transport: wraps a non-reflect codec
            // `Arc`, so it cannot be a bsx literal — a template provides it,
            // expanding at build to leave no marker to re-fire on reload.
            .register_template::<AgentSocket>()
            // ends each heading's drive after its budget, so the robot steps then stops.
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

/// How a heading maps onto the wheels: the drive-step tuning, authorable from
/// the scene next to the `apply-heading` route (bare numbers coerce in each
/// unit's authoring form, mm/s and deg/s; durations from strings):
///
/// ```xml
/// <Route path="apply-heading" {(ApplyHeading, DriveStepConfig{duration: "0.5s"})}/>
/// ```
///
/// Fields default sensibly, so a scene sets only what it tunes.
#[derive(Debug, Clone, Copy, Component, Reflect)]
#[reflect(Component, Default)]
#[type_path = "perceive_act"]
pub struct DriveStepConfig {
    /// Forward speed of a `Forward` heading (matches the wgpu body's step).
    pub speed: LinearVelocity,
    /// Turn rate of a `Left`/`Right` heading (positive = left, spin in place).
    pub turn_rate: AngularVelocity,
    /// How long a heading drives before the robot stops. A heading is a discrete,
    /// bounded step (matching the wgpu fox), not the RC routes' continuous drive,
    /// so the robot never runs away after the agent's loop ends.
    pub duration: Duration,
}

impl Default for DriveStepConfig {
    fn default() -> Self {
        Self {
            speed: LinearVelocity::from_mm_per_sec(60.0),
            turn_rate: AngularVelocity::from_deg_per_sec(90.0),
            duration: Duration::from_millis(800),
        }
    }
}

/// Serve `apply-heading`: map the chosen [`Heading`] onto the Alvik's
/// [`DifferentialDrive`] and stamp a [`DriveStep`] deadline; [`expire_drive_step`]
/// zeroes the drive when it expires, so a heading is a bounded step and the robot
/// halts between commands. Tuning comes from the route entity's
/// [`DriveStepConfig`], falling back to the defaults.
///
/// The reply carries [`ApplyHeadingReply::settle`] (the step budget), and the
/// agent's `respond-multi-modal` sleeps that long before finishing, so the next
/// photo waits for the robot to stop. The wait lives agent-side because this
/// handler's beet task has no async-timer waker (the countdown rides bevy
/// [`Time`] in the schedule), and holding the reply on a bridged poll starves
/// the schedule loop.
///
/// Bound with `<Route path="apply-heading" {ApplyHeading}/>`. The robot is found
/// by a global `With<AlvikRobot>` query, so the socket client need not be nested
/// under the robot.
#[action(route, handler_only)]
#[derive(Default, Clone, Component, Reflect)]
#[reflect(Component)]
#[type_path = "perceive_act"]
async fn ApplyHeading(cx: ActionContext<ApplyHeadingInput>) -> Response {
    let heading = cx.input.heading;
    let config = cx
        .caller
        .get_cloned::<DriveStepConfig>()
        .await
        .unwrap_or_default();
    let (linear, angular) = match heading {
        Heading::Forward => (config.speed, AngularVelocity::default()),
        Heading::Left => (LinearVelocity::default(), config.turn_rate),
        Heading::Right => (LinearVelocity::default(), -config.turn_rate),
    };
    info!(
        "apply-heading: {:?} -> ({} mm/s, {} deg/s) for {:?}",
        heading,
        linear.as_mm_per_sec(),
        angular.as_deg_per_sec(),
        config.duration
    );
    // set the velocity on the robot root and return its entity, skipping cleanly if
    // the Alvik is not up yet (the model trails several round-trips, so in practice
    // it is).
    let world = cx.world();
    let robot = world
        .with_state::<Query<(Entity, &mut DifferentialDrive), With<AlvikRobot>>, _>(
            move |mut drives| {
                let (entity, mut drive) = drives.iter_mut().next()?;
                drive.linear = linear;
                drive.angular = angular;
                Some(entity)
            },
        )
        .await;
    let settle = match robot {
        Some(robot) => {
            let _ = world
                .entity(robot)
                .insert(DriveStep {
                    timer: Timer::new(config.duration, TimerMode::Once),
                })
                .await;
            config.duration
        }
        // no robot up: nothing will move, so nothing to settle
        None => Duration::ZERO,
    };
    Response::ok_json(&ApplyHeadingReply { settle }).unwrap_or_else(|_| Response::ok())
}

/// Marks the robot as mid-step; [`expire_drive_step`] counts it down and stops the
/// drive.
#[derive(Debug, Clone, Component, Reflect)]
#[reflect(Component)]
struct DriveStep {
    /// Counts down the heading's drive budget.
    timer: Timer,
}

/// End each heading's drive once its [`DriveStep`] timer finishes, zeroing the
/// [`DifferentialDrive`] so a heading is a bounded step and the robot halts between
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
            drive.linear = LinearVelocity::default();
            drive.angular = AngularVelocity::default();
            commands.entity(entity).remove::<DriveStep>();
        }
    }
}

/// The direction to head next, mirroring the agent's `Heading` wire type. The wire
/// is JSON, so the externally-tagged unit variants (`"Forward"`) match byte for
/// byte without sharing the (std-only) `beet_extra` definition.
#[derive(
    Debug,
    Default,
    Clone,
    Copy,
    PartialEq,
    Component,
    Reflect,
    serde::Deserialize,
    serde::Serialize,
)]
#[reflect(Component, Default)]
enum Heading {
    /// Drive straight ahead.
    #[default]
    Forward,
    /// Turn to the left.
    Left,
    /// Turn to the right.
    Right,
}

/// The `apply-heading` tool input, mirroring the agent's `ApplyHeadingInput`. The
/// blanket `FromRequest for T: DeserializeOwned` extracts it from the JSON body.
#[derive(Default, Reflect, serde::Deserialize, serde::Serialize)]
struct ApplyHeadingInput {
    /// The direction to head next.
    heading: Heading,
}

/// The `apply-heading` reply, mirroring the agent's `ApplyHeadingReply`: how long
/// the commanded step runs, so the agent-side `respond-multi-modal` can wait out
/// the movement.
#[derive(Default, Reflect, serde::Deserialize, serde::Serialize)]
struct ApplyHeadingReply {
    /// How long the drive step runs before the robot halts.
    settle: Duration,
}
