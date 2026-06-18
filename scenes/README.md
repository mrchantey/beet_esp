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

## Authoring widgets

The scenes read as behaviour trees because `beet_esp` registers a set of
non-generic component tags (the generic primitives `Repeat`/`Sequence`/`Script`
cannot resolve from a bare tag). See `src/scene.rs` and `src/alvik/scenes.rs`:

- `<RouteAction path="..">` — a behaviour-tree route (`SpawnAction` + path); its
  child tree runs when the route is called.
- `<Loop>` — repeat the child forever (`Repeat`). `<Steps>` — run children in
  order (`Sequence`). `<Wait ms={50}/>` — pass after a delay (`EndInDuration`).
- `<At path="..">` — spread onto a direct route handler to bind its path.
- `<LedScript rhai="..">` / `<AlvikScript rhai="..">` — run a rhai program each
  tick over the WS2812 / the robot.
- `<RoombaStep/>`, `<LineFollowStep/>`, `<Drive linear={..} angular={..}/>`,
  `<DriveRoute/>`, `<LedRoute/>` — the Alvik leaves and route handlers.

## The scenes

- `led-script.bsx` — bare-ESP32 WS2812 driven by a rhai colour program (the
  default firmware).
- `roomba.bsx`, `line-follower.bsx` — Alvik wander / line-follow loops.
- `dance-routine.bsx` — Alvik forward/turn/forward/stop, once.
- `script.bsx` — Alvik controller as a rhai program over the sensors.
- `rc.bsx` — Alvik remote control: `drive/:dir` and `led/:side/:state` routes.

The Alvik scenes need the `alvik` firmware build
(`cargo run --no-default-features --features alvik,router,wifi,rhai`); they parse
on any build but their leaves only run with the robot attached.
