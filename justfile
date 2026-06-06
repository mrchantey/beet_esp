# beet_esp scene-server workflows.
#
# The firmware (`cargo run`) is a scene server; the host `beet` CLI loads scenes
# onto it. The scene commands `beet` uses live upstream in beet_router and are
# exported to `beet.json` by the beet-cli `default_cli` example. `beet` reads the
# device address from `BEET_REMOTE_URL` in this directory's `.env`, so the
# `beet load|run|dump|clear|reset` commands target the device directly.

beet_dir := "/home/pete/me/worktrees/beet/apps/beet"

# List recipes.
default:
    @just --list

# Regenerate ./beet.json: the scene the `beet` CLI loads to become a scene
# controller. Re-run after changing the upstream scene commands.
beet-json:
    cd {{beet_dir}} && cargo run -p beet-cli --example default_cli -- --output {{justfile_directory()}}/beet.json

# Install the `beet` CLI (with the scene-control commands registered).
install-cli:
    cd {{beet_dir}} && cargo install --path crates/beet-cli

# Flash + monitor the scene-server firmware on the device. Detaches after 30s.
run:
    timeout -s INT 30s cargo run --release

# Dump the canonical example scenes as JSON over defmt; copy each into scenes/.
export-scenes:
    timeout -s INT 30s cargo run --release --example export_scenes
