---
name: healthcheck
description: >-
  Measure and reason about memory headroom on the ESP32-S3 — heap, stack, and
  how close a build is to the hardware ceiling. Use when asked to check memory
  health, hunt a leak, decide if a feature combination "fits", size the heap, or
  understand an out-of-memory crash. Walks through the `HealthPlugin` reporting
  system and the kitchen-sink / linker-map method for finding the real ceiling.
---

# Memory healthcheck

How to find out whether a `beet_esp` firmware is comfortable or about to fall off
a cliff. Written for someone fluent in software but newer to embedded: the
mental model is the unfamiliar part, the procedure is easy.

## The mental model (read this first if you come from servers)

On a server you rarely think about memory layout: virtual memory is effectively
infinite, the stack grows on demand, and if you leak the OOM killer eventually
reaps you. **None of that is true here.** The ESP32-S3 has a fixed ~512 KB of
on-chip SRAM, no MMU, no swap, no OS to clean up after you. Three pools share
that SRAM, and you must keep each one inside its lines yourself:

- **Heap** — backs `Box`, `Vec`, `String`, and the whole Bevy `World`. Unlike a
  server, the heap is a **fixed-size region you choose at compile time** (the
  `heap_allocator!` / `init_esp!(heap_size: …)` number). Allocate past it and the
  allocator returns null → an allocation-error handler fires → the program dies.
  There is no "ask the OS for more".
- **Stack** — function call frames + locals. It grows *downward* from a fixed top
  toward a fixed bottom. Overrun the bottom and you smash whatever's there
  (silent corruption) unless a guard catches it. There is no "grow the stack".
- **Statics** (`.data`/`.bss`) — globals, `static` buffers, and the heap buffer
  itself. Fixed at link time.

A few terms you'll see:

- **SRAM / DRAM segment** — the physical on-chip RAM. The linker carves it into
  named regions (`dram_seg`, `dram2_seg`).
- **defmt / RTT** — the logging path. `defmt` is a compact log format; RTT is a
  channel `probe-rs` reads over the debug USB. `info!`/`warn!` end up on your
  terminal via this.
- **probe-rs** — flashes the chip and streams the RTT log. `cargo run` invokes it.
- **Watermark / stack painting** — a trick to measure peak stack use; explained
  below.
- **Guard** — a canary word the runtime (`esp-rtos`) places near the stack bottom
  to detect overflow.

The goal of a healthcheck: confirm the heap isn't **leaking** (creeping up over
time) and that neither heap nor stack is **near its ceiling**.

## The tooling: `HealthPlugin`

`src/health.rs` provides a Bevy plugin that does all the measuring. Add it right
after `Esp32Plugin`:

```rust
App::new()
    .add_plugins((Esp32Plugin, HealthPlugin, /* LedPlugin, WifiPlugin… */))
    .run();
```

It takes three kinds of snapshot:

1. **boot** — captured by `init_esp!` (inside `#[beet_esp::main]`) *before*
   `App::new()`, when the heap is empty. This is also where it **paints the
   stack** (see below). Stored in a static.
2. **pre-startup** — taken in `PreStartup`, after the chip/embassy are up but
   before any app `Startup` system. The baseline for leak comparisons.
3. **periodic** — every 2 s in `Update`, with deltas vs the last report and vs
   pre-startup.

### Reading a report

```
── health #14 (up 26 s) ──
HEAP INFO
Size: 172044
Current usage: 165556
Max usage: 169172
Total freed: 275072
Total allocated: 440628
Memory Layout:
Internal | ██████████████████████████████████░ | Used: 99% (Used 98080 of 98300, free: 220)
Internal | ███████████████████████████████░░░░ | Used: 91% (Used 67476 of 73744, free: 6268)
stack: peak 15664 / 166732 B used, min headroom 151068 B
Δ heap: 0 B since last, 59756 B since pre-startup
```

- **Current usage / Size** — live heap bytes / total heap. The two `Internal |`
  bars are the two heap *regions* (see "Why region 1 hits 99%" below).
- **Max usage** — high-water mark of the heap. The real ceiling test: compare
  this to `Size`, not `Current usage`. Here 169172 of 172044 → it peaked at
  ~98%, under 3 KB to spare.
- **Total allocated / freed** — lifetime churn. Big numbers that *track each
  other* (440 K alloc vs 275 K freed, net = current) are healthy: it means lots
  of alloc/free cycling (e.g. sockets) with no accumulation.
- **stack: peak X / total** — `X` is the **all-time deepest** stack use, not the
  current depth. `total` is the stack region size. `min headroom` is the closest
  the stack ever came to the bottom guard.
- **Δ heap since last** — the leak signal. **Flat (`0`) = healthy.** A one-time
  step then flat (the usual startup → steady-state pattern) is fine. Only a
  *sustained* climb across reports is a leak; the plugin warns after 3 growing
  reports in a row.

### How stack painting works (the watermark trick)

There's no counter for "how much stack did we use". So at boot, `health.rs` fills
the entire free stack with a known sentinel word (`0x5A5A5A5A`). Later it scans
up from the bottom and counts surviving sentinels — the first overwritten word
marks the deepest the stack ever reached. This gives an **all-time peak**, not
just the instantaneous depth you'd see by sampling. It measures the **CPU0
main-task stack**, which the embassy executor and every bridge driver share.
(Wi-Fi/BLE worker threads get their own heap-allocated stacks, so their cost
shows up under the heap instead.)

## Procedure: a per-example healthcheck

1. **Source the toolchain** (every shell): `. $HOME/export-esp.sh`
2. **Build lean** — only the features the example needs, so you're measuring the
   real footprint, not a kitchen sink:
   ```shell
   cargo build --release --no-default-features --features led --example blinky
   ```
3. **Flash + stream with a timeout** — `cargo run` never returns on its own
   (it attaches the RTT monitor forever), so wrap it:
   ```shell
   timeout -s INT 30s cargo run --release --no-default-features --features led \
       --example blinky > /tmp/blinky.log 2>&1
   ```
   30 s captures ~13 reports (one per 2 s) — enough to see the heap go flat.
4. **Read the trend**, not a single report:
   ```shell
   grep -nE "health #|Current usage|Max usage|stack:|Δ heap|WARN" /tmp/blinky.log
   ```
   Healthy = `Δ heap … 0 B since last` repeating, `Max usage` well under `Size`,
   stack `peak` a small fraction of `total`.

Watch for: a non-zero `Δ since last` that recurs (leak), `Max usage` approaching
`Size` (heap ceiling), or `min headroom` shrinking toward 0 (stack ceiling).

## Measuring real headroom: the kitchen-sink method

"Are we near the ceiling?" has two answers, and they're very different:

- **Soft ceiling** — the `heap_size` you configured (default 172 KB total). A
  report showing 99% is hitting *this*. It's a number you picked, not the chip.
- **Hard ceiling** — physical SRAM. Found from the linker map + the binary's
  section sizes.

To answer the hard question, **combine everything that will ever run together**
and measure, then compare against the silicon budget. `examples/kitchen_sink.rs`
is exactly this: LED + Wi-Fi (client *and* server) + reflection-backed
`world_serde`, all on one `World`, under `HealthPlugin`.

### Step 1 — measure the combined working set

```shell
cargo build --release --no-default-features --features led,wifi --example kitchen_sink
timeout -s INT 40s cargo run --release --no-default-features --features led,wifi \
    --example kitchen_sink > /tmp/ks.log 2>&1
grep -nE "Max usage|Used:|stack: peak" /tmp/ks.log | tail
```

This tells you the **peak heap** the combination actually needs (e.g. ~183 KB).
Note: the heap bump in that example (`#[beet_esp::main(heap_size = 176 * 1024)]`)
is required — the default 172 KB can't hold the combo (see "failure mode").

### Step 2 — find the silicon budget (linker map)

The DRAM regions are defined per-chip in esp-hal's `ld/<chip>/memory.x`:

```shell
HAL=$(ls -d ~/.cargo/registry/src/*/esp-hal-*/ | tail -1)
grep -A2 "dram_seg\|dram2_seg" "$HAL/ld/esp32s3/memory.x"
```

For the ESP32-S3:

| Region      | Bytes   | Holds                                            |
| ----------- | ------- | ------------------------------------------------ |
| `dram_seg`  | 341,760 | `.data` + `.bss` (incl. the heap buffer) + stack |
| `dram2_seg` | 73,744  | the "reclaimed" heap (heap **region 2**)         |

Total app-usable internal DRAM ≈ **406 KB**. (The rest of the 512 KB is
instruction cache, IRAM startup code, and ROM-reserved — not yours.)

### Step 3 — see where it actually went (section sizes)

`size -A` on the ELF breaks the binary into sections. Use the esp toolchain's
`size`, or stock `llvm-size`:

```shell
SIZE=$(find ~/.rustup/toolchains/esp -name 'xtensa-esp-elf-size' | head -1)  # or: llvm-size
ELF=/home/pete/.cargo_target/xtensa-esp32s3-none-elf/release/examples/kitchen_sink
"$SIZE" -A "$ELF" | grep -E '\.data|\.bss|\.stack|\.dram2'
```

For kitchen_sink:

```
.data         14640
.data.wifi      496
.bss         201588   ← includes the 180,224 B heap buffer
.stack        83404   ← but the watermark says only ~16 KB is ever used
.dram2_uninit 73744   ← heap region 2
```

So **real** statics (everything in `.data`/`.bss` *except* the heap buffer) ≈
36.5 KB. The heap buffer is just a big `static` living in `.bss`.

### Step 4 — compute the ceiling

The stack is the key insight: it's sized at **83 KB here (210 KB for LED-only
builds!)** but the watermark proves only **~16 KB** is ever used. That idle stack
RAM is reclaimable into the heap. So:

```
max main heap = dram_seg − fixed_overhead − safe_stack
              = 341,760 − ~45,000 (gap + real statics) − 32,768 (generous 32 KB stack)
              ≈ 264,000 B
total heap ceiling = max main heap + dram2_seg(73,744) ≈ 337,000 B ≈ 329 KB
```

Verdict for "everything at once": **183 KB used / ~329 KB ceiling ≈ 56%.** Not
close to the silicon — the binding limit is the `heap_size` you set, and you can
nearly double it in software by reclaiming over-provisioned stack.

### Step 5 — reclaim the headroom (if a build needs it)

Just raise the heap; the stack region automatically shrinks to whatever's left,
and the watermark confirms it's still plenty:

```rust
#[beet_esp::main(heap_size = 176 * 1024)]   // 96 KB default → 176 KB
```

Re-run the healthcheck and confirm `min headroom` is still large (kitchen_sink:
83 KB stack, 67 KB headroom — fine for a 16 KB peak).

## The failure mode (important)

When a build needs more heap than configured, it does **not** degrade
gracefully. Running kitchen_sink at the default 96 KB heap produced **zero log
output** — it ran out of memory *during `App` construction*, before the first
health print. So overshooting the heap looks like a board that boots to silence.

This is why the boot/pre-startup snapshots matter: **the last successful line
tells you how far it got.** If you see the boot snapshot but never pre-startup,
the World construction OOM'd. If you never see boot, suspect download mode (next
section), not memory.

## Gotchas

- **Why region 1 hits 99% while region 2 is empty.** The allocator fills regions
  in order; it only spills into region 2 (the reclaimed `dram2_seg`) once region 1
  is full. A scary-looking "region 1 99%" with an empty region 2 is normal — read
  `Max usage` vs total `Size` for the true picture.
- **`internal-heap-stats`.** `Max usage` / `Total allocated` / `Total freed` only
  exist because `esp-alloc` is built with the `internal-heap-stats` feature (set
  in `Cargo.toml`). Without it you only get current usage. The overhead is an
  O(regions) tally per alloc — negligible.
- **Sticky download mode.** Rapidly reflashing (especially right after an OOM
  crash) can leave the S3 stuck in ROM **download mode**: `probe-rs` flashes fine
  but no defmt ever appears (it scans for RTT forever), and `attach` reads
  garbage. This is *not* a firmware bug. Fix: physical cold boot — unplug the USB
  port ~10 s, leave the `COM` port out, replug `USB`. See the esp-rust
  `troubleshooting.md` "sticky download mode" entry. A silent app from an OOM
  looks identical, so rule out memory (last successful log line) before assuming
  download mode.
- **Escape hatches beyond internal SRAM.** The linker reserves a ~64 KB DCache
  region that can become a 3rd heap region (esp-alloc supports up to 3). And R8
  modules have octal **PSRAM** (2–8 MB) usable via `psram_allocator!` — slower,
  and with the Xtensa "no atomics in PSRAM" caveat (see CLAUDE.md). Neither is
  wired up by default; reach for them only when internal SRAM genuinely runs out.

## Worked numbers (ESP32-S3 DevKit, this repo)

| Build         | Features  | Pre-startup heap | Heap peak | Heap region | Stack peak | Verdict |
| ------------- | --------- | ---------------- | --------- | ----------- | ---------- | ------- |
| blinky        | led       | —                | ~104 KB   | 172 KB      | 13.0 KB    | healthy, flat |
| world_serde   | (none)    | 91 KB            | 102 KB    | 172 KB      | 13.6 KB    | healthy, flat (59%) |
| behavior_tree | action    | 139 KB           | 172 KB    | 270 KB (bumped) | 16.0 KB | flat (64%); async runtime is heap-hungry |
| ecs_wifi      | wifi      | 102 KB           | 164 KB    | 172 KB      | 16.9 KB    | flat, but tight (95%) |
| kitchen_sink  | led,wifi  | 106 KB           | 178 KB    | 254 KB (bumped) | 16.9 KB | flat, ~54% of silicon |

Stack is never the constraint (peaks 13–17 KB against 80–220 KB regions). Heap is
the thing to size; leaks would show as a non-zero, recurring `Δ since last`.

### The beet async runtime (beet_action) is the heaviest per-feature heap cost

`behavior_tree` vs `world_serde` is a clean A/B: both are core-only with no radio,
differing only in the `action` feature (beet_action + the beet_async bridge +
bevy's `TaskPoolPlugin` + ~20 reflected control-flow type registrations). Adding
it costs, on the same baseline:

- **pre-startup heap +48 KB** (91 → 139 KB) — standing up `ActionPlugin` /
  `AsyncPlugin` / `TaskPoolPlugin` and registering the reflected action types,
  before any tree runs.
- **peak heap +70 KB** (102 → 172 KB) — running the tree spawns entities,
  bridge futures, cached `SystemState`s, and the task-pool executors. This
  roughly **doubles** the core peak and forces a `heap_size` bump (96 → 192 KB
  main) — the default 96 KB main heap OOMs during `App` construction.
- **flash `.text` +350 KB** (651 → 1003 KB) — code, not RAM, but worth noting.
- statics +5 KB, stack +2 KB — both negligible.

So if a build pulls in beet actions, budget ~140 KB of heap before it does
anything and bump the main heap accordingly. Wi-Fi, by contrast, is cheaper at
rest (+11 KB pre-startup) but spikes ~58 KB on connect (DHCP/TCP buffers), which
is what pushes `ecs_wifi` to 95% of a default region.

### Are we near any limit?

No build is near the **silicon** ceiling (~329 KB usable internal heap; see the
kitchen-sink method above): the worst case is kitchen_sink at 178 KB ≈ 54%.
Stack is nowhere close either — every build peaks 13–17 KB against regions of
80–220 KB (the region auto-shrinks as you raise `heap_size`, reclaiming the
idle stack RAM). The only builds that get *tight* do so against the **soft**
ceiling (the `heap_size` you picked): `ecs_wifi` at 95% of a default 172 KB
region is the one to watch — it has ~8 KB to spare and would benefit from a
modest bump. Everything else has comfortable headroom.
