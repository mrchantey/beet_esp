# beet_esp scene-server workflows.
#
# The firmware (`cargo run`) is a scene server; the host `beet` CLI loads scenes
# onto it. The scene commands are built into `beet` itself (see beet-cli's
# `main`). `beet` reads the device address from `BEET_REMOTE_URL` in this
# directory's `.env`, so `beet load|run|dump|clear|reset` target the device.

beet_dir := "/home/pete/me/worktrees/beet/apps/beet"

# List recipes.
default:
    @just --list

# Install the `beet` CLI (scene-management commands built in).
install-cli:
    cd {{beet_dir}} && cargo install --path crates/beet-cli

# Flash + monitor the scene-server firmware on the device (the fast iterate
# loop). Detaches after 30s. The dev profile is tuned for iteration: opt-level
# "s" + fat LTO keep the image small (so it flashes fast) while debug-assertions
# and overflow-checks stay on, so panics are caught on device. Use this for
# day-to-day work; `run-release` builds the shippable image.
run:
    timeout -s INT 30s cargo run

# Flash + monitor the shippable release image (assertions off, smallest binary).
run-release:
    timeout -s INT 30s cargo run --release

# Generate the canonical example scenes as JSON into target/scenes/ (gitignored).
# The scene types live in this crate; the `scenes` host crate builds them on the
# PC (no device needed) and writes each file directly.
export-scenes:
    cd scenes && cargo run
