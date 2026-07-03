# templates

Hand-authored beet `.bsx` documents, split by what they drive. Scene templates
(`esp32/`, `alvik/`) are pushed to the device over the wire and installed on
`/load` (beet's `TemplateLoader` dispatches `.bsx` bytes to its BSX engine), no
build step: edit a file and push it. Infra workflows (`infra/`) are the firmware
dev loop expressed as `<Command>` shell-outs, reused by `../main.bsx`.

```sh
beet load templates/alvik/dance-routine.bsx   # push a scene (BEET_REMOTE_URL targets the device)
beet run dance-routine                         # call the route the scene installed
beet dump                                      # print the device's current scene
beet clear                                     # despawn it + reset the hardware
```

## Layout

- `esp32/` scenes for a bare ESP32 breakout (no robot):
  - `led-script.bsx` on-board WS2812 driven by a script colour program (the
    default firmware).
- `alvik/` scenes for the Arduino Alvik robot. They parse on any build but their
  leaves only run on the `alvik` firmware with the robot attached:
  - `dance-routine.bsx` forward / turn / forward / stop, once.
  - `roomba.bsx`, `line-follower.bsx` wander / line-follow loops.
  - `script.bsx` a controller scripted over the sensor snapshot.
  - `rc.bsx` remote control: `drive/:dir` and `led/:side/:state` routes.
- `infra/` the firmware dev loop, each a `<Command>` run as a `{BehaviorSequence}`
  route by `../main.bsx`:
  - `build.bsx` compile the Alvik firmware (`cargo build --release --features alvik`).
  - `flash.bsx` build + flash over the USB-JTAG probe (probe-rs).
  - `monitor.bsx` tail the running firmware's RTT log output (probe-rs attach).

## Authoring scenes

Scenes are authored straight from beet's upstream primitives, no non-generic alias
components stand in for them:

- `<Repeat>` / `<Sequence>` repeat the child forever / run children in order. The
  generic `Repeat<()>` / `Sequence<(), ()>` resolve from a bare tag (each is the
  sole registered instantiation).
- `<EndInDuration duration="50ms"/>` a behaviour-tree leaf that passes after the
  delay. The duration coerces from a unit string (`"50ms"`, `"1s"`) by beet's BSX
  engine; the unit is required.
- `<RouteAction path="..">` install a behaviour-tree route (`PathPartial` +
  `SpawnAction`); its child tree runs when the route is called.

The firmware adds a few domain widgets (see `src/scene.rs`, `src/alvik/scenes.rs`,
`src/alvik/routes.rs`):

- `<LedScript script="..." language="rhai">` / `<AlvikScript script="..." language="rhai">`
  script leaves run each tick over the WS2812 / the robot. `language` selects the
  backend (rhai or quickjs), falling back to the build default when absent.
- `<Route path="drive/:dir" {DriveHandler}/>` / `<Route path="led/:side/:state" {LedHandler}/>`
  bind a direct route handler to a path.
- `<RoombaStep/>`, `<LineFollowStep/>`, `<SetDrive linear={..} angular={..}/>` the
  Alvik behaviour-tree leaves.

## Authoring infra workflows

The `infra/` files use the upstream `<Command>` action (a `beet_action`
behaviour-tree leaf that shells out, streaming output live and failing on a
non-zero exit): `exe` is required, `args` / `cwd` / `env` optional. `cwd` is an
absolute path so the workflow resolves the same wherever it is invoked from (run
from this repo via `../main.bsx`, or referenced from the talk demo via
`<Template src>`). `../main.bsx` wires each as a `<Route path=".." {BehaviorSequence}>`,
and runs `flash` followed by `monitor`.
