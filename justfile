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
# scene-management + Command/BehaviorSequence capabilities (`just install-cli`).

beet_dir := "/home/pete/me/worktrees/beet/apps/beet"

# List recipes.
default:
    @just --list

# Install the `beet` CLI (its `SceneManagementPlugin` registers the scene-push
# commands, and `BehaviorSequence`/`Command` back the build/flash/monitor verbs
# `main.bsx` wires). Re-run after pulling beet changes.
install-cli:
    cd {{beet_dir}} && cargo install --path crates/beet-cli

# On-device unit tests (beet's own harness over semihosting).
test:
	. $HOME/export-esp.sh && cargo test -p beet_esp --lib
