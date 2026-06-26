# scenes

Hand-authored beet scenes as `.bsx` files, pushed to the device over the wire.
Each is a [BSX](https://github.com/mrchantey/beet) document the firmware parses
on `/load` (beet's `TemplateLoader` dispatches `.bsx` bytes to its BSX engine),
installing the route it carries. There is no build step: edit a file and push it.

```sh
beet load scenes/roomba.bsx    # push (BEET_REMOTE_URL targets the device)
beet run roomba                # call the route the scene installed
beet dump                      # print the device's current scene
beet clear                     # despawn it + reset the hardware
```

## Authoring

The scenes are authored straight from beet's upstream primitives — no non-generic
alias components stand in for them:

- `<Repeat>` / `<Sequence>` — repeat the child forever / run children in order. The
  generic `Repeat<()>` / `Sequence<(), ()>` resolve from a bare tag (each is the
  sole registered instantiation).
- `<EndInDuration duration="50ms"/>` — a behaviour-tree leaf that passes after the
  delay. The duration coerces from a unit string (`"50ms"`, `"1s"`) or a bare number
  of milliseconds, by beet's BSX engine.
- `<RouteAction path="..">` — install a behaviour-tree route (`PathPartial` +
  `SpawnAction`); its child tree runs when the route is called.

The firmware adds a few domain widgets (see `src/scene.rs`, `src/alvik/scenes.rs`,
`src/alvik/routes.rs`):

- `<LedScript script="..." language="rhai">` / `<AlvikScript script="..." language="rhai">`
  — script leaves run each tick over the WS2812 / the robot. `language` selects the
  backend (rhai or quickjs), falling back to the build default when absent. Authoring
  templates over `Script` + the domain step, mirroring upstream's `ScriptRoute`.
- `<DriveRoute path="drive/:dir"/>` / `<LedRoute path="led/:side/:state"/>` — bind a
  direct route handler to a path (templates that insert the `PathPartial`).
- `<RoombaStep/>`, `<LineFollowStep/>`, `<Drive linear={..} angular={..}/>` — the
  Alvik behaviour-tree leaves.

## The scenes

- `led-script.bsx` — bare-ESP32 WS2812 driven by a script colour program (the
  default firmware).
- `roomba.bsx`, `line-follower.bsx` — Alvik wander / line-follow loops.
- `dance-routine.bsx` — Alvik forward/turn/forward/stop, once.
- `script.bsx` — Alvik controller as a script over the sensors.
- `rc.bsx` — Alvik remote control: `drive/:dir` and `led/:side/:state` routes.

The Alvik scenes need the `alvik` firmware build (`cargo run --release --features
alvik`); they parse on any build but their leaves only run with the robot attached.
