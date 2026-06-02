//! # alvik — a remote-control CLI for the RC Alvik
//!
//! The native counterpart to the `alvik-rc` firmware example. Built on the same
//! routing stack as beet's `examples/todo/todo.rs`: each subcommand is a route
//! and the positional arguments form the path. Where todo's routes mutate a
//! document, these routes **fetch the robot** — each action issues an HTTP
//! request to the Alvik's static address and prints its reply.
//!
//! It is structured as a router (not a flat `match`) so it can grow into a full
//! app later; for now every route is a thin async action around [`Request::send`].
//!
//! ## Building & running
//!
//! This crate is *not* the ESP32 firmware: a sibling `rust-toolchain.toml` +
//! `.cargo/config.toml` pin the host toolchain/target so it builds natively
//! rather than for `xtensa-esp32s3`. From this directory:
//!
//! ```sh
//! cargo run -- drive forward      # forward | back | left | right | stop
//! cargo run -- led left on        # side: left|right   state: on|off
//! cargo run -- drive stop         # always stop when done
//! ```
//!
//! Against the `alvik-scene` firmware the routes themselves arrive over the
//! wire. Send a scene, then drive it:
//!
//! ```sh
//! cargo run -- load rc             # install the drive/led routes (drive/led now work)
//! cargo run -- load dance-routine  # install the dance action route
//! cargo run -- run dance-routine   # fire it (returns at once; robot dances)
//! cargo run -- dump                # print the loaded scene as JSON
//! cargo run -- clear               # despawn the scene + reset
//! ```
//!
//! `load <name>` resolves `<name>` to `scenes/<name>.json` (override the folder
//! with `ALVIK_SCENES_DIR`) or takes a literal path. The robot URL defaults to
//! [`DEFAULT_URL`]; override with `ALVIK_URL`, eg
//! `ALVIK_URL=http://192.168.86.50:8080 cargo run -- drive forward`.

use beet::prelude::*;

/// Where the robot lives by default — the static IP the `alvik-rc` firmware
/// binds. Override at runtime with the `ALVIK_URL` env var.
const DEFAULT_URL: &str = "http://192.168.86.222:8080";

#[beet::main]
async fn main() -> Result {
	// Router-as-CLI: the parsed request path selects a route, exactly as todo.rs
	// does — only here the routes reach across the network instead of a document.
	let mut world = (AsyncPlugin, RouterPlugin).into_world();
	let root = world
		.spawn((default_router(), children![
			exchange_route("drive/:dir", Drive),
			exchange_route("led/:side/:state", Led),
			// scene control: load a scene by name/path, then clear/reset/dump it
			// and fire any action route the scene installed.
			exchange_route("load/:scene", LoadScene),
			exchange_route("clear", Clear),
			exchange_route("reset", Reset),
			exchange_route("dump", Dump),
			exchange_route("run/:route", Run),
		]))
		.flush();
	world.update_local();

	// A CLI invocation is just a request: the positional args become the path,
	// which the router matches to a route (`alvik drive forward` -> /drive/forward).
	let request = Request::from_cli_args(CliArgs::parse_env());
	let response = world
		.entity_mut(root)
		.call::<Request, Response>(request)
		.await?;

	cross_log!("{}", response.text().await?);
	Ok(())
}

/// The robot's base URL, from `ALVIK_URL` or [`DEFAULT_URL`].
fn alvik_url() -> String {
	env_ext::var("ALVIK_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

/// Directory holding the example scene files, from `ALVIK_SCENES_DIR` or the
/// sibling `scenes/` folder next to this crate.
fn scenes_dir() -> String {
	env_ext::var("ALVIK_SCENES_DIR")
		.unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../scenes").to_string())
}

/// Resolve a scene argument to a file path: an existing path as-is, else
/// `<scenes_dir>/<name>.json`.
fn scene_path(name: &str) -> String {
	if std::path::Path::new(name).exists() {
		name.to_string()
	} else {
		format!("{}/{name}.json", scenes_dir())
	}
}

/// `drive <forward|back|left|right|stop>` — forward the direction to the robot's
/// `/drive/:dir` route and return its reply.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Drive(cx: ActionContext<RequestParts>) -> Result<Response> {
	let dir = cx.input.get_param("dir").unwrap_or("stop").to_string();
	Request::get(format!("{}/drive/{dir}", alvik_url()))
		.send()
		.await
}

/// `led <left|right> <on|off>` — forward to the robot's `/led/:side/:state` route.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Led(cx: ActionContext<RequestParts>) -> Result<Response> {
	let side = cx.input.get_param("side").unwrap_or("right").to_string();
	let state = cx.input.get_param("state").unwrap_or("off").to_string();
	Request::get(format!("{}/led/{side}/{state}", alvik_url()))
		.send()
		.await
}

/// `load <scene>` — POST a scene file to the robot's `/load`, replacing its
/// current routes. `<scene>` is a path or a name under the scenes dir, eg
/// `alvik load dance-routine`.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn LoadScene(cx: ActionContext<RequestParts>) -> Result<Response> {
	let scene = cx.input.get_param("scene").unwrap_or("rc").to_string();
	let path = scene_path(&scene);
	let body = std::fs::read(&path)
		.map_err(|err| bevyhow!("cannot read scene {path}: {err}"))?;
	Request::post(format!("{}/load", alvik_url()))
		.with_content_type(MediaType::Json)
		.with_body(body)
		.send()
		.await
}

/// `clear` — despawn the loaded scene and reset the robot.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Clear(_cx: ActionContext<RequestParts>) -> Result<Response> {
	Request::get(format!("{}/clear", alvik_url())).send().await
}

/// `reset` — stop the motors and turn the UI LEDs off.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Reset(_cx: ActionContext<RequestParts>) -> Result<Response> {
	Request::get(format!("{}/reset", alvik_url())).send().await
}

/// `dump` — print the currently loaded scene as JSON.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Dump(_cx: ActionContext<RequestParts>) -> Result<Response> {
	Request::get(format!("{}/dump", alvik_url())).send().await
}

/// `run <route>` — fire an action route the loaded scene installed, eg
/// `alvik run dance-routine`.
#[action(handler_only)]
#[derive(Default, Clone, Component)]
async fn Run(cx: ActionContext<RequestParts>) -> Result<Response> {
	let route = cx.input.get_param("route").unwrap_or("").to_string();
	Request::get(format!("{}/{route}", alvik_url())).send().await
}
