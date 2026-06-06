# beet_esp scene-server workflows.
#
# The firmware (`cargo run`) is a scene server; the host `beet` CLI loads scenes
# onto it. The remote commands that `beet` uses live upstream in beet_router and
# are exported to `beet.json` by the beet-cli `remote_loader` example.

beet_dir := "/home/pete/me/worktrees/beet/apps/beet"
# Where this device lives on the network (the static IP the firmware binds).
device_url := "http://192.168.86.222:8080"

# List recipes.
default:
    @just --list

# Regenerate ./beet.json: the remote-loader scene the `beet` CLI loads to turn
# itself into a remote control for this device. Re-run after changing the
# upstream remote commands.
beet-json:
    cd {{beet_dir}} && cargo run -p beet-cli --example remote_loader -- --output {{justfile_directory()}}/beet.json

# Install the `beet` CLI (with the remote scene-control commands registered).
install-cli:
    cd {{beet_dir}} && cargo install --path crates/beet-cli

# Flash + monitor the scene-server firmware on the device. Detaches after 30s.
run:
    timeout -s INT 30s cargo run --release

# Load a scene file onto the device, eg `just load scenes/led-script.json`.
load scene:
    SCENE_URL={{device_url}} beet load {{scene}}

# Fire an action route the loaded scene installed, eg `just run-route led-script`.
run-route route:
    SCENE_URL={{device_url}} beet run {{route}}

# Print the currently loaded scene as JSON.
dump:
    SCENE_URL={{device_url}} beet dump

# Despawn the loaded scene and reset the device.
clear:
    SCENE_URL={{device_url}} beet clear
