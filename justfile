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

# Flash + monitor the scene-server firmware on the device. Detaches after 30s.
run:
    timeout -s INT 30s cargo run --release

# Dump the canonical example scenes as JSON over defmt; copy each into scenes/.
# The scene types live in this no_std crate, so the scenes are built on-chip.
export-scenes:
    timeout -s INT 30s cargo run --release --example export_scenes
