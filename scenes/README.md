# Scenes

A beet *scene* is a reflection-serialized slice of the ECS that the firmware
(`cargo run`) loads over HTTP to become its live API. A scene is not config data,
it *is* the device's routes and behaviours. Send a different scene and the same
firmware exposes a different device.

## Generating the scenes

This directory is a host crate that generates the canonical example scenes. The
scene-definition types live in `beet_esp` (its ECS components, `#[action]`s and
scene bundles), which compiles for the host without its `device` hardware stack,
so the scenes are built on the PC rather than on the ESP32:

```sh
cd scenes && cargo run   # or: just export-scenes
```

It writes each scene to `../target/scenes/<name>.json`. Those files are
gitignored and regenerated on demand (handy after changing a route component),
not committed. The entity ids in them are placeholders, remapped on load; only
the `ChildOf` wiring between them matters.

## Sending a scene to the device

Send one with the upstream `beet` CLI. The scene commands live in beet_router and
are exported to `beet.json` by the beet-cli `default_cli` example; `just beet-json`
regenerates it and `just install-cli` installs `beet`. The CLI reads the device
address from `BEET_REMOTE_URL` in this project's `.env`:

```sh
beet load target/scenes/led-script.json   # POST it to /load
beet run led-script                        # fire the action route it installed
beet dump                                  # print the loaded scene as json
beet clear                                 # despawn it + reset
```

or with curl (the firmware serves on beet's `DEFAULT_SERVER_PORT`, 8337):

```sh
curl --data-binary @target/scenes/led-script.json \
     -H 'content-type: application/json' \
     http://192.168.86.222:8337/load
```

## The scenes

### Generic (bare ESP32)

- **led-script.json** — `led-script` runs a **rhai program** (`Script`) every
  100 ms on the on-board WS2812: it reads `input.elapsed_ms` + `input.led` (the
  current colour, packed `0xRRGGBB`) plus its own `state` map, and returns
  `#{ led, state }`. Edit the `source` string to reprogram the LED with no
  reflash. Served by the default firmware (`led,router,wifi,rhai,device`).

### Alvik (`--features alvik`)

- **rc.json** — the plain remote-control API: `drive/:dir` and `led/:side/:state`
  wired to the `DriveRoute` / `LedRoute` components. Equivalent to the hard-coded
  `alvik-rc` example, but sent over the wire.
- **dance-routine.json** — an *action route*: `dance-routine` runs a `Sequence`
  behaviour tree (forward 1s, left 1s, forward 1s, stop) via `ApplyDrive` +
  `EndInDuration`. Firing it returns immediately; the tree plays out on the async
  pool.
- **line-follower.json** — `line-follower` repeats a bang-bang `LineFollowStep`
  every 50 ms (forward on white, steer right on black).
- **roomba.json** — `roomba` repeats `RoombaStep`: cruise until the centre ToF
  sees a wall within 20 cm, then spin to clear it.
- **script.json** — `script` runs a **rhai program** (`Script` + `AlvikScriptStep`)
  every 100 ms: it reads an Alvik sensor snapshot (depth, line, yaw, touch,
  elapsed) plus its own `state` map, and returns drive + LED + next `state`.
  Requires the firmware built with `--features alvik,router,rhai,device`.
