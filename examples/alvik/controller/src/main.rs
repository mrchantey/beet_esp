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
//! The robot URL defaults to [`DEFAULT_URL`]; override with `ALVIK_URL`, eg
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
