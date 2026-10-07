# beet_esp scene-server workflows.
#
# The whole dev loop runs through the host `beet` CLI against this directory's
# `main.bsx` (discovered by walking up from the cwd). One verb per invocation:
#
#   beet build                                    # compile the Alvik firmware
#   beet flash                                    # build + flash over USB, then monitor
#   beet monitor                                  # tail the firmware's RTT output
#   beet load templates/alvik/dance-routine.bsx   # push a `.bsx` scene to the device
#   beet run dance-routine                        # call a route the scene installed
#   beet dump                                     # print the device's current scene
#   beet clear                                    # despawn the scene + reset the hardware
#
# The firmware lifecycle verbs (build/flash/monitor) are reusable `<Command>`
# workflows in `templates/infra/`; the scene verbs push over HTTP to the device at
# `BEET_REMOTE_URL` (this directory's `.env`). Requires a `beet` built with the
# scene-management + Command/ExchangeSequence capabilities (`just install-cli`).

beet_dir := "/home/pete/me/beet"

# List recipes.
default:
    @just --list

# Install the `beet` CLI (its `SceneManagementPlugin` registers the scene-push
# commands, and `ExchangeSequence`/`Command` back the build/flash/monitor verbs
# `main.bsx` wires). Re-run after pulling beet changes.
install-cli:
    cd {{beet_dir}} && cargo install --path crates/beet-cli


beet:
	cargo run --manifest-path=/home/pete/me/beet/crates/beet-cli/Cargo.toml

# On-device unit tests (beet's own harness over semihosting).
test:
	. $HOME/export-esp.sh && cargo test -p beet_esp --lib

# Every build target without a board: the on-device test binary, both firmwares,
# each example under exactly its `required-features`, and the host-compiled
# scene types. Fails on the first error or warning.
check-all:
	#!/usr/bin/env bash
	set -euo pipefail
	. $HOME/export-esp.sh
	gate() {
		echo "cargo $*"
		log=$(cargo "$@" 2>&1) || { echo "$log"; exit 1; }
		if grep -qE '^(warning|error)' <<<"$log"; then echo "$log"; exit 1; fi
	}
	gate test -p beet_esp --lib --no-run
	gate build
	gate build --no-default-features --features device,alvik,router,wifi,quickjs
	gate build --no-default-features --features device,led --example blinky --example led_scan
	gate build --no-default-features --features wifi,action --example http_client --example http_server
	gate build --no-default-features --features wifi,action,sockets --example socket_client
	gate build --no-default-features --features ble --example bluetooth-client
	gate build --no-default-features --features mdns --example mdns_server --example mdns_browser
	gate build --no-default-features --features led,wifi,action --example kitchen_sink
	gate build --no-default-features --features device,action --example behavior_tree
	gate build --no-default-features --features device,router,wifi --example ecs_router
	gate build --no-default-features --features device --example world_serde
	gate build --no-default-features --features random --example random
	gate build --no-default-features --features clock --example clock
	gate build --no-default-features --features device,alvik --example alvik-codec-selftest --example alvik-blinky --example alvik-wheels --example alvik-drive --example alvik-servo --example alvik-sensors --example alvik-line-follower --example alvik-touch-move
	gate build --no-default-features --features device,alvik,router,wifi --example alvik-rc
	gate build --no-default-features --features device,quickjs --example scripting --example scripting_watchdog --example scripting_bench
	gate check -p beet_esp --target x86_64-unknown-linux-gnu -Zbuild-std=std,panic_abort --no-default-features --features led,alvik,router
	gate check -p beet_esp --target x86_64-unknown-linux-gnu -Zbuild-std=std,panic_abort --no-default-features
	echo "PASS check-all"
