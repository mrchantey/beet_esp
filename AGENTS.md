# beet_esp


ESP32-S3 embedded firmware for the [beet](https://github.com/mrchantey/beet)
project. `no_std` Rust on `esp-hal`.

Always begin a conversation with 'gday pete'.

## Context

This is a downstream library of the primary beet project at `/home/pete/me/beet`, depended on by path. Beet's conventions are inherited through the synced block at the bottom of this file (refreshed by beet's `sync-downstream` skill); where this header conflicts with the block, the header wins. You have permission to make changes as required, but do not commit them, so the user can review.

## Configuration

Set at generation time — don't change without a reason.

- **Chip:** ESP32-S3 (Xtensa). Target: `xtensa-esp32s3-none-elf`.
- **Environment:** `no_std`, `esp-hal` 1.1.
- **Async:** embassy via `esp-rtos`.
- **Connectivity:** `esp-radio` — Wi-Fi + BLE (`trouble-host`), COEX enabled.
- **Heap:** `esp-alloc` (two heaps; the second adds RAM for Wi-Fi/BLE COEX).
- **Logging:** `log`/`tracing` over RTT. **Panic handler:** the crate's own RTT
  handler (`beet_esp` lib `#[panic_handler]`, which the on-device test build
  swaps for a semihosting-exit handler so a failed test reports rather than hangs).
- **Flash/debug:** `probe-rs` (S3 native USB JTAG, no external probe needed).
- **Tests:** beet's own on-hardware harness (`beet_core::testing` via the
  `testing_embedded` feature, registered with `linkme`), run with
  `cargo test -p beet_esp --lib`. See `src/device_test.rs`.

The exact generator options are recorded in a `generator parameters:` comment at
the top of `src/main.rs`.

## First-time machine bootstrap

A fresh machine has none of the toolchain. Verified from a clean state
2026-07-02. Every step is user-level (`~/.cargo`, `~/.rustup`) except the udev
rule, which is the one and only `sudo` step.

```shell
# 1. Xtensa Rust toolchain + ~/export-esp.sh (large download, ~1-2 GB)
cargo binstall -y espup && espup install

# 2. Flash/debug tooling (probe-rs, cargo-flash, cargo-embed)
cargo binstall -y probe-rs-tools

# 3. sccache. The global ~/.cargo/config.toml sets `rustc-wrapper = "sccache"`,
#    so builds die with "could not execute process `sccache`" without it.
cargo binstall -y sccache

# 4. udev rule so probe-rs can open the USB-JTAG as a normal user (THE sudo step).
#    Without it probe-rs errors "failed to open device (errno 13)". Note `probe-rs
#    list` still works read-only, so a missing rule looks fine but is not:
#    flashing needs write access.
#
#    The upstream probe-rs rule file matches with ATTRS{} (device-or-parent) and
#    did NOT fire for the ESP32-S3 on this machine's udev (verified with
#    `udevadm test`: the file is read but no rule matches, MODE stays 0664, no
#    uaccess tag). Match the device's OWN attrs (ATTR) instead. GROUP="wheel"
#    grants access directly (pete is in wheel) so it does not depend on
#    logind/seat; uaccess is a bonus for the active desktop session.
echo 'SUBSYSTEM=="usb", ATTR{idVendor}=="303a", ATTR{idProduct}=="1001", MODE="0660", GROUP="wheel", TAG+="uaccess"' \
  | sudo tee /etc/udev/rules.d/70-esp32s3-jtag.rules
sudo udevadm control --reload-rules
sudo udevadm trigger --action=add --attr-match=idVendor=303a
# GROUP="wheel" is applied by udev directly, so no physical replug is needed for
# access. (uaccess only re-applies on a real device add/replug.)
```

Confirm access with `probe-rs info --chip esp32s3`: the node becomes
`group=wheel crw-rw----` and it attaches (`Xtensa Chip IDCODE ...`).

```shell
# 5. Wi-Fi credentials. The firmware reads BEET_WIFI_SSID / BEET_WIFI_PASSWORD
#    via env! at compile time (build.rs exposes them from .env), so the build
#    fails without them. Copy the template and fill in your network.
cp .env.example .env   # then edit BEET_WIFI_SSID / BEET_WIFI_PASSWORD
```

Note the build honours a global `CARGO_TARGET_DIR` if set (this machine points it
at `~/.cargo_target`), so the firmware ELF lands there, not in the project-local
`target/`.

## Environment setup (required before building)

The Xtensa toolchain comes from `espup`, not stock rustup. Every shell needs the
env vars sourced:

```shell
. $HOME/export-esp.sh   # sets LIBCLANG_PATH etc.
```

The toolchain is pinned in `rust-toolchain.toml` (the `esp` channel). Build
failures with linker or libclang errors are almost always a missing
`export-esp.sh`.

## Commands

```shell
cargo build --release    # compile
cargo run   --release    # flash + monitor (probe-rs runner)
cargo test               # on-hardware tests (embedded-test)
```

Target, runner and `build-std` are configured in `.cargo/config.toml`.

**`cargo run` does not exit.** The probe-rs runner flashes and then attaches an
RTT/`defmt` monitor that streams output indefinitely, it never returns on its
own. When running non-interactively (e.g. from an agent), always wrap it in a
timeout so it detaches after capturing output.

**Size the timeout to clear the flash first.** The timeout also kills the
erase/program/verify that runs *before* any RTT appears, so too short a window
aborts mid-program and leaves the chip in an unknown state (a silent app that
looks like dead hardware). The full `alvik` release build (~2.5 MiB) takes
~100-110s to flash before it streams, so 30s is only safe for re-attaching to an
already-flashed chip. Give a fresh flash a generous window:

```shell
timeout -s INT 30s  cargo run --release                     # re-attach only (no reflash)
timeout -s INT 240s cargo run --release --features alvik     # fresh flash: ~2 min to program, then streams
```

## Gotchas

- **Don't `mem::forget` esp-hal drivers** — their `Drop` resets the peripheral
  and cancels in-flight DMA; forgetting leaves hardware in a bad state.
- `esp-hal`'s core is 1.0-stable, but **most ancillary crates and many peripheral
  drivers are unstable and not covered by SemVer**. Pin dependencies; `cargo
  update` can break unstable features. Read migration guides between releases.
- Per-chip API docs: <https://docs.espressif.com/projects/rust/> — pick ESP32-S3.

## Hardware

Dual-port ESP32-S3 DevKit, brought up and verified 2026-05. Day-to-day:

- **Use the `USB` port** (native USB-Serial-JTAG `303a:1001`, what `probe-rs`
  drives), not `COM` (a CH340 UART bridge — serial only, no JTAG).
- **Keep `COM` unplugged while using `probe-rs`** — its auto-reset lines can tug
  `GPIO0`/`EN` into download mode.
- Once the udev rule from "First-time machine bootstrap" is installed, `probe-rs
  list` shows `ESP JTAG -- 303a:1001` and can flash. Note enumeration works
  read-only even without the rule, so `probe-rs list` succeeding does not by
  itself prove flashing will work.
- **On-board addressable LED (WS2812) is on `GPIO48`**, driven over RMT. See
  `examples/blinky.rs` (RGB hue fade) and `examples/led_scan.rs` (the GPIO
  scanner that found it).
- **Two+ boards at once:** each appears in `probe-rs list` with its own serial;
  the `.cargo/config.toml` runner sets no `--probe`, so target one explicitly,
  e.g. `probe-rs run --probe 303a:1001:<SERIAL> --chip esp32s3 … <elf>`.
- **Board in `lsusb` as `303a:4001` but missing from `probe-rs list`:** its
  firmware grabbed the USB-OTG port as a CDC serial (`/dev/ttyACM*`), hiding the
  native JTAG. Enter download mode (hold `BOOT`, tap `RST`) to restore
  `303a:1001`, then flash and cold-boot.
- **alvik**: If there's an Arduino Alvik plugged in, it's safe to assume that it is in an upright postion and wheels are not touching the ground so you should freely test motors etc as needed.

If a flash succeeds but no `defmt` ever appears (probe-rs scans for RTT
forever), the chip is stuck in download mode: **the app isn't running, so don't
mistake it for a dead peripheral** (a silent app looks exactly like a broken
LED/sensor). Cold-boot (unplug ~10s, leave `COM` out, replug `USB`) before you
start debugging hardware. See the "sticky download mode" entry in trouibleshooting.

## Deeper reference

A full `esp-rust` skill (project generation, hardware setup, esp-hal coding
patterns, troubleshooting) lives in-repo at `.agents/skills/esp-rust/`.

All troubleshooting is located at `.agents/skills/esp-rust/troubleshooting.md`



## Verification


After making code changes, we need to verify on device if it's plugged in. Ensure everything is compiling, re-upload to the device which is plugged in, and verify all good

<!-- beet:sync:begin — beet's AGENTS.md, refreshed by the sync-downstream skill; do not hand-edit -->
# Agent Instructions

You are the coding agent for the beet project. Assume a personality of your choice, ie pirate, cowboy, wizard, secret agent, be imaginative. Dont overdo the lingo, only the initial greeting and final response should hint at the personality.

Beet is a pre-release (no current users) malleable engine built on the bevy game engine, in the lineage of user-modifiable software like smalltalk and hypercard.

## Core Principles

1. Beet is entirely configurable. Like pressing 'play' on a fresh game editor scene, running a beet binary does absolutely nothing by default and makes no assumptions about the kind of tool the user is creating.
2. Beet is target agnostic. Everything everything everything. Http servers run on wasm, tui servers run on ssh etc. Use `AncestorQuery<&BlobStore>` instead of `fs_ext`. In general `FsStore` must only be inserted explicitly in tests.

## This file

Every agent reads this file, so it keeps only what every session needs and stays under ~15KB: a new subsystem gets one pointer line below, its cheatsheet in its crate docs, any procedure in a skill. `CLAUDE.md` is a symlink to this file.

Situational cheatsheets, read before touching the subsystem:

- Actions: one-per-entity, overloads, providers, `#[field]`, facets: `crates/beet_action/README.md`
- Servers and the lifecycle verbs: `crates/beet_net/README.md`
- Cloud resources: stacks, grants, buckets, jobs: `crates/beet_infra/README.md` + `.agents/skills/infra-deploy`
- The beet CLI, entries, wasm binaries, making any binary a beet runtime: `crates/beet-cli/README.md` + `crates/beet_router/src/launch/mod.rs`
- Styling: `crates/beet_ui/src/style/mod.rs`
- Scene editing (tree, inspector, entity and component pickers): `crates/beet_ui/src/widgets/scene_editor/mod.rs`
- Rendering (web + charcell): `.agents/skills/rendering`
- Mermaid diagrams (fences, `<Mermaid>`, modes, the Auto rule, the paint tokens): `crates/beet_ui/src/parse/mermaid/mod.rs`

## Workflow

- when provided a plan or list of work to do, just do it! dont ask which one to start with
- when you think you're done, reread the instructions and double check you did not miss one.

## Context

- There is no time constraint. Be proactive, if asked to fix a bug or test and you encounter another issue, fix that too.
- Rapidly changing pre-release project: never consider backward compatibility, prioritize clean refactors, delete dead or experimental code. We never mark `#[deprecated]`, replace the machinery instead.
- Prefer iterative approaches: try something, learn from it, try again. Search the codebase as-needed instead of preloading everything.
- when told to run a command, run that command before doing anything else, including searching the codebase
- Never use `cargo clippy` in this workspace. Never run `cargo clean` without permission, rebuilds take hours.
- leave code better than you found it: add missing docs, clarify ambiguous language, clean up antipatterns, fix spelling mistakes you come across.
- Be fearless pushing changes upstream and generalizing patterns. If a type would reasonably always be used with another, wire it directly and massage it into `Reflect` (or `pub(crate)` with a public template) rather than reaching for a wrapper:
	- bad: `#[template] pub fn BazzTemplate() -> impl Bundle { (Bazz, BazzAction) }`
	- good: make `Bazz` `#[require(BazzAction)]` and use `<Bazz/>` directly
- Do not create non-doc examples without being explicitly asked.
- Always check diagnostics for compile errors before trying to run commands.
- We do not use `tokio`, always the `async-` equivalents, ie `async-io`, `async-task`.

## Memory

Never use `.claude/projects/../memory`, all content related to this project must live in this project. The only place you are permitted to persist memory is in `./agent/memory`.

## Conventions

- A rust module reads like a good book: public high level structs at the top, implementation details below. Mod files are just reexports; prefer splitting into specific sub files, but dont 'create a fresh file' because the one you're working on is messy.
- Respond to the user with a single numbered sequence with strictly one point per number. intersperse with subheadings as required but subheadings do not get numbers. Open questions should have available options listed alphabetically , with a) being selected by default if no answer.

Example
```md
## Subheading foo
1. some info about this point...
## Subheading bar
2. some info about another point that needs a decision..
	- a) do foo
	- b) do bar
	- c) do bazz
3. some more info..
```
- Functions longer than ~20 lines may have brief comments describing each step.
- Never insert arbitrary ie 80 col manual reflow newlines in markdown documents.
- all shared dependencies are declared in the workspace Cargo.toml; if one needs no-default-features, disable that at the workspace level and reenable as required
- for reserved keyword idents, use escaping `r#struct`, never misspelling `strukt`  
- Beet is cross-platform: use `fs_ext`, `env_ext` instead of `std::fs`/`std::env`, adding missing methods as needed.
- The one canonical store an app runs from is the **repo store**, marked `RepoStore` and enforced as one per world: `RepoStore` for types, `repo_store` for idents, "repo store" in prose (never "site"/"entry"/"app" store). Every other `BlobStore` is a plain store, named by a `StoreRef` or scoped out of an ancestor by a `DirPath`. Its deploy-side declaration is the store block carrying `RepoStoreBlock` (`<S3BucketBlock label="repo" {RepoStoreBlock}/>`), found by type through `RepoStoreQuery`, never by label.
- Never scatter new env vars: config flows through request params, a route declaring its flags on its own `Reflect` params type behind `ParamsPartial` so `--help` documents them. `BootstrapConfig` describes ONE process launch: read with `BootstrapConfig::get()`, construct only to launch another process (`ChildProcess::with_bootstrap`).
- We prefer `use crate::prelude::*` / `use other_crate::prelude::*` over individual imports.
- Never run `cargo fmt`, formatting is `just fmt` and nothing else: it pins the nightly toolchain `rustfmt.toml` requires and passes `--all`; bare `cargo fmt` silently reformats the tree into a huge bogus diff.
- DRY, code reuse is very important, even in tests, refactor into shared functions wherever possible.
- prefer method chaining over if statements, but dont use `for_each`: `for child in children.iter().filter(..)` is correct.
- Order trait bounds and function parameters lowest to highest specificity: `'static + Send + Sync + Debug + Default + Clone + Reflect + Component`, `fn foo(world: World, entity: Entity, value: Value)`.
- Never mention agent plans or temporary tasks in code docs.
- `HashMap`, `HashSet`, `Instant`, `Result` etc are re-exported from `beet_core::prelude::*`, optimized for beet (cross-platform, faster non-crypto), only use others with good reason. Prefer `SmolStr` for strings likely to be small.
- Always use `bevyhow!{}`, `bevybail!{}` unless a consumer needs the error type, then `thiserror` (now no_std). Never wrap errors (`.map_err(|e| bevyhow!("{e}"))?`): `BevyError` implements `From<E: Error>`, just use `?`.
- Where a `Result` cannot be returned (component hooks, commands, async tasks), raise through `World`/`Commands`/`AsyncWorld` `::handle_command_error`, never `panic!`, `debug_assert!` or a bare `error!`; a hook reaches it via `DeferredWorld::commands()`.
- Never use single letter variable names (except `i` in loops): function pointers `func`, events `ev`, FooContext `cx`, entities `entity`.
- Continue `long().method().chains()` rather than storing temporaries; the `xtend.rs` blanket traits assist: `.xmap()` is `.map()` for any type, `bar(bazz).xmap(foo)` not `foo(bar(bazz))`, `.xok(foo)` not `Ok(foo)`.
- Getters/setters: prefer the `#[derive(Get,Set,SetWith)]` macros over manual implementation; adjust the macros to suit new usecases if required.
- Utility modules have the `_ext` suffix, are reexported as `pub mod`, and callers keep the qualifier: `async_ext::do_async_thing().await`.
- Free items: a top-level `pub fn`/`pub const`/`pub static` is permitted only in a `*_ext` module, a sanctioned namespace module (ie `js_runtime::cwd()`), a `#[template]` constructor, or generated code; everything else is an associated item on its type, or not pub (`pub(crate)`/private are fine). Bevy systems and observers stay free fns but private, registered by their plugin. Visibility is private until needed, for types as well as functions. Audit recipe: `.agents/skills/audit-free-fns`.
- git: never create branches or make commits unless explicitly told to, whatever the checkout state; keep things as unstaged changes.
- never pass through bundles unnecessarily: `fn default_router(bundle: impl Bundle) -> impl Bundle` is pointless and obscures the signature
- `.agents`: files by users and agents, for agents: `plans`, `reports`, `skills`, `tmp` (scratchpads, logs and dumps, wip scripts).
- Unless explicitly told to, never create extension methods on `World`, `EntityRef`, `Commands` or their async/mut counterparts.
- Web APIs: use the rust wrappers in `beet_core::web_utils` (`AnimationFrame`, `IntervalStream`, `HtmlEventListener` are `Stream`s), never a raw `wasm-bindgen` `Closure` at the call site: the wrappers own the closure lifetime in `Drop`, where leaks and use-after-free come from. A missing wrapper is a reason to add one.

## Documentation

- Quality over quantity, documentation and comments must be as short and concise as possible:
	- good: `// run launch step if no match`
	- bad: `// if there is not a match for the hash then we should run the launch step`
- doctests: `ignore` is an absolute last resort (macros); prefer helper methods that let a doctest run over `no_run`, though `no_run` is sometimes required, ie network requests.
- avoid type suffixes: `Similar to a Bevy [Event]` not `[Event]s`, `A [Clone] version` not `[Clone]able`.
- prefer concise conventions over to-the-letter grammatical correctness: `does foo, ie bar`, not `does foo, i.e., bar`.

## Testing

- We use the custom `beet_core::testing` runner and matchers in all crates; all tests use `#[beet_core::test]` (inside `beet_core` itself, `#[crate::test]`, see its Cargo.toml).
- This workspace is massive: never run entire workspace tests, always specify the crate (`cargo test -p beet_core`), and use `tail` to avoid context bloat (always with `just test-all`).
- wasm tests: beet cannot run doctests, so always specify `--lib` or `--test` for wasm
- for complex output use snapshot testing, `.xpect_snapshot()`, updating with the `--snap` flag
- unit tests belong at the bottom of the file; the need for integration tests is rare
- Quality over quantity, only test what needs testing (not accessors or builders). Do not add a `test` prefix to function names: `adds_numbers`, not `test_adds_numbers`.
- Matchers chain: `some().long().chain().xpect_contains("foo").xnot().xpect_contains("bar")`. They are not a replacement for `.unwrap()`: always `.unwrap()`/`.unwrap_err()` when you just want the value.
- scene tests: `scene_ext::test_world()` (the minimal scene plugin set), insert required resources, then `world.spawn_scene(rsx!{ <div/> }).unwrap()`
- by default only test files are logged; use `--log-cases` to see individual cases

## Debugging

- The two main causes of ECS bugs are (1) missing components: an entity lacked what a system or observer expected, and (2) incorrect traversals: a traversal assuming a structure a refactor has changed. Inspect with `world.log_component_names(entity)`.
- The `related!` and `children!` macros are *set* not *insert* instructions, clobbering any existing relations.
- never use `println!`, it is silent in wasm. Informational logging uses the `log` macros `error!`/`warn!`/`info!`/`debug!`; `cross_log!` is ONLY for output that must not carry a log prefix (a streamed response body, the program's actual result). Temp dumps: `foo.xprint()`; control-flow log points: `breakpoint!()`.
- In wasm, `app.run()` immediately returns `AppExit::Success`; use `app.run_async()` to run to completion.
- when a bug is found in actual usage of a feature (examples, `site/`), it is not enough to fix it: isolate it, understand it and add tests to avoid regression.

## Bevy Cheatsheet

- Observers can accept closures capturing their environment, systems cannot: use input parameters, `fn my_system(foo: In<Foo>, ..)`.
- prefer `world.spawn((Parent, children![(Child, ..)]))` over a second spawn with `ChildOf`, unless the child entity needs tracking.
- Formalize any remotely complex traversal as a `SystemParam` (see `card_query.rs`) or use the existing helpers (`AncestorQuery`, ..); avoid traversing with world directly, use `world.run_system_once(..)` or the often more ergonomic `world.with_state::<MyQuery>(|my_query| ..)`.
- Prefer `Populated` over `Query`, which skips the system when the query is empty; for an 'any of these queries' pattern use `.run_if(|a, b| !a.is_empty() || !b.is_empty())`.
- A `#[template]` is a constructor returning `impl Bundle` (or `()` for effects, or `Result<impl Bundle>`), not a UI-only thing; `#[template(system)]` takes `SystemParam`s and does arbitrary ECS work at build time. Prefer a `<MyThing/>` template over a reflect-marker + `On<Insert>` observer: it expands away at build, leaving no component to re-fire on scene reload.
- Component hooks: `#[component(on_add = ...)]` accepts a call yielding a closure: use the constructors in `beet_core::bevy_utils::hook_ext`, `observe(my_observer)` for observers watching the entity, `entity_hook(|entity| ..)` for `EntityCommands` work.
- A command aimed at an entity another task may despawn (a server's connections) must tolerate its absence: `try_insert`, `try_remove`, `try_trigger_target`. `EntityWorldMut::despawn` flushes the queue *after* removing the entity, so an observer's deferred command routinely lands on a gone target, a panic under the default error handler.

## BSX Cheatsheet

- **Every entity is authored under the tag of the type it most *is*.** `<div>`/`<span>` mean "this paints as a box of text" and are never a generic carrier to hang the real type off a spread. This applies in every position: a behavior loop is `<Repeat>`, a thread is `<Thread>`, a route is `<Route>`; whatever is left over rides a `{spread}` on that same entity.
	- bad: `<div {(Route{path:"deploy"}, ExchangeSequence)}>`; good: `<Route path="deploy" {ExchangeSequence}>`
	- between co-located types the entity's **action** wins (one action per entity, see `beet_action`): `<Repeat {RunThread}>`, not `<RunThread {Repeat}>`. Absent an action, the noun the entity names wins: `<Thread {(Sequence, FsStore{path:".."})}>`.
- A generic type resolves by base name to its sole registered instantiation, as a tag exactly as in a spread, so `<Repeat>`, `<Sequence>` and `<RepeatTimes total_times=2>` all author directly.
- For a plain grouping use `<Fragment>`, not `<div>`: it carries spreads, directives and children but emits no element. (`<Template>` is the *include* front-end, `<Template src="..">`; with no `src` a directives-only no-op.)
- `<Tag/>` resolves a component/template by short type path and spawns its own entity; `{Spread}` / `{(A, B)}` adds components to the *current* entity. String attributes coerce to the field type (`SmolStr`, `Duration` from `"30s"`, `Option<T>`, enum unit variants), so a reflect component is usually authorable without a template.
- A `<Tag>`'s children land as its direct children (slots are transparent), so a child-reading handler like `{ExchangeSequence}` (a sequenced route) reads them: `<Route path="deploy" {ExchangeSequence}><MyBlock/><MyAction/></Route>`.
- **Features remove components, never entities — unless the node is an effect.** By default a document loads whole in every binary: an unregistered uppercase tag warns, marks its entity `UnregisteredTag` and still builds its directives, spreads and children, so a lean binary's tree keeps its shape with only behavior missing. A sequence route that skipped every child fails naming unregistered tags, and `beet check` elevates `UnregisteredTag` to an error.
- **`bx:cfg` is the one exception, and the only exclusion mechanism.** The markup twin of Rust's `#[cfg]`, with Rust's operators and precedence: `bx:cfg="(feature:infra && feature:extra) || feature:all"` removes its node and subtree from the SYNTAX TREE before the build walk, so nothing downstream observes it. It exists because some nodes are effects rather than structure (`<Template src>` reads a file) and an effect has no inert form. It leaves a `CfgExcluded` tombstone carrying the condition and the element's `path`, so an excluded route still exists and reports why rather than 404ing. Atoms are `namespace:argument` resolved through the `BsxConditions` seam (`feature:`, `version:`, `env:` in core; downstream registers its own), and an unknown namespace is a hard error, never a silent false. `<RequireCfg cfg=".."/>` is the same condition asserted rather than applied: it refuses the whole load, naming every unmet atom at once. `allow_unregistered` opts out a tag whose whole content is the missing behavior (`<LiveReloadScript/>`).
- A `Router` is a **url space**: its subtree's routes root at it (no ancestor `<Route>` prepends) and it owns its own `RouteTree`, so a whole site mounts under a command route with urls still rooted at `/`, and a dispatching surface can never reach routes outside its namespace. Resolve with `RouteTree::of` (a server's tree lives on its `Router` child), not `entity.get::<RouteTree>()`.
<!-- beet:sync:end -->
