# beet_esp scene-server workflows.
#
# The firmware (`cargo run`) is a scene server; the host `beet` CLI pushes `.bsx`
# scenes onto it. The scene commands (`load`/`run`/`dump`/`clear`/`reset`) are
# wired as routes by this directory's `main.bsx`, which `beet` discovers by
# walking up from the cwd. `beet` reads the device address from `BEET_REMOTE_URL`
# in this directory's `.env`, so the commands target the device:
#
#   beet load scenes/roomba.bsx   # push a `.bsx` scene
#   beet run roomba               # call a route the scene installed
#   beet dump                     # print the device's current scene
#   beet clear                    # despawn the scene + reset the hardware

beet_dir := "/home/pete/me/worktrees/beet/apps/beet"

# List recipes.
default:
    @just --list

# Install the `beet` CLI (its `SceneManagementPlugin` registers the scene-push
# commands `main.bsx` wires).
install-cli:
    cd {{beet_dir}} && cargo install --path crates/beet-cli

# Flash + monitor the scene-server firmware on the device (the fast iterate
# loop). Detaches after 30s. The dev profile is tuned for iteration: opt-level
# "s" + fat LTO keep the image small AND the relink fast (measured on hardware:
# fat LTO collapses bevy/beet's monomorphised surface, so the linker has less to
# do and the image flashes faster; dropping LTO nearly doubles both). debug-
# assertions and overflow-checks stay on, so panics are caught on device. Use
# this for day-to-day work; `run-release` builds the shippable image.
run:
    timeout -s INT 30s cargo run

# Flash + monitor the shippable release image (assertions off, smallest binary).
run-release:
    timeout -s INT 30s cargo run --release

# Push a scene from `scenes/` to the device, eg `just load roomba`. The firmware
# receives the `.bsx` over `/load` and parses it (TemplateLoader dispatches `.bsx`
# bytes to the BSX engine), installing the route it carries.
load scene:
    beet load scenes/{{scene}}.bsx

test:
	. $HOME/export-esp.sh && cargo test -p beet_esp --lib
