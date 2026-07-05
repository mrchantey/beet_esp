//! The embassy↔ECS transport: typed sync→async pipes, a request/reply RPC type,
//! plus a type-erased spawner for long-lived embassy drivers.
//!
//! Every async driver shares one shape: a task that pulls a message from a
//! sync→async pipe, `.await`s on hardware/IO, and optionally pushes results back
//! through another pipe. The LED is the simplest case (pipe = [`Latest`],
//! message = `Color`, await = `Ws2812::write`, no result). When a side needs a
//! *reply* — hand a request across the boundary and await its response — use
//! [`AsyncBridge`], which bundles a [`Queue`] of inputs with a one-shot reply
//! per call. All drivers spawn the same way via [`spawn_driver`].
//!
//! ## The toolkit
//!
//! Above the primitives sit two reusable glue helpers, one per shape, so a new
//! bridged capability is "provide the worker / producer" rather than a
//! hand-written loop and drain:
//!
//! - **Request/reply** ([`AsyncBridge`]). The embassy responder is always
//!   `loop { recv; reply.send(worker(job).await) }`; [`run_worker`] /
//!   [`spawn_worker`] stand that up from a `worker` (the Wi-Fi client). When the
//!   *ECS* is the responder instead (the Wi-Fi server: embassy initiates, the
//!   world answers), [`drain_to_async`] is the per-frame mirror.
//! - **Notification** ([`Queue`]). An embassy producer pushes events into a
//!   `static` queue and the world reacts; [`drain_to_observers`] is the per-frame
//!   ECS drain that `commands.trigger`s each item to observers.
//!
//! The LED is the counter-example: it awaits [`Latest`] and writes RMT, never
//! touching `&mut World`, so it needs no ECS-side helper at all.

use alloc::boxed::Box;
use core::future::Future;
use core::pin::Pin;
use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

/// Latest-wins, lossy, coalescing pipe for *state outputs* (LED colour, servo
/// setpoint). Sending again before the reader wakes drops the older value, so
/// the driver only ever sees the newest state.
///
/// `const`-constructible so it can live in a `static`.
pub struct Latest<T: Send> {
    inner: Signal<CriticalSectionRawMutex, T>,
}

impl<T: Send> Latest<T> {
    pub const fn new() -> Self {
        Self {
            inner: Signal::new(),
        }
    }

    /// Publish the newest value, replacing any not-yet-received one.
    pub fn send(&self, value: T) {
        self.inner.signal(value);
    }

    /// Await the next published value.
    pub async fn recv(&self) -> T {
        self.inner.wait().await
    }

    /// Take the pending value if one exists, without awaiting.
    pub fn try_recv(&self) -> Option<T> {
        self.inner.try_take()
    }
}

/// FIFO, bounded, lossless pipe for *discrete work* (e.g. Wi-Fi requests). When
/// full, [`send`](Self::send) drops the *newest* item (it hands the rejected
/// value back) rather than blocking, keeping producers non-async.
///
/// `const`-constructible so it can live in a `static`.
pub struct Queue<T, const N: usize> {
    inner: Channel<CriticalSectionRawMutex, T, N>,
}

impl<T, const N: usize> Queue<T, N> {
    pub const fn new() -> Self {
        Self {
            inner: Channel::new(),
        }
    }

    /// Enqueue without blocking. On a full queue the value is rejected and
    /// returned in `Err` (drop-newest).
    pub fn send(&self, value: T) -> Result<(), T> {
        self.inner.try_send(value).map_err(|e| match e {
            embassy_sync::channel::TrySendError::Full(v) => v,
        })
    }

    /// Await the next queued item.
    pub async fn recv(&self) -> T {
        self.inner.receive().await
    }

    /// Take the next queued item if one is ready, without awaiting.
    pub fn try_recv(&self) -> Option<T> {
        self.inner.try_receive().ok()
    }
}

/// A request/reply pipe across the embassy↔ECS boundary, for the cases where a
/// no-reply [`Latest`]/[`Queue`] is not enough: one side hands an `In` across
/// and must `.await` the matching `Out` back.
///
/// Direction-agnostic — it does not care which executor initiates. Whichever
/// side originates the work calls [`call`](Self::call); the other side drains
/// ([`recv`](Self::recv) for an embassy task, [`try_recv`](Self::try_recv) for a
/// per-frame Bevy system) and answers via the [`Reply`] handle. `Out` is opaque:
/// a caller that treats errors as values uses `Out = Result<_, _>`, one that
/// folds errors into the payload uses the payload type directly.
///
/// Built on the existing [`Queue`] plus a one-shot [`Signal`] per in-flight
/// call, and `const`-constructible so it can live in a `static`.
pub struct AsyncBridge<In, Out, const N: usize> {
    queue: Queue<Exchange<In, Out>, N>,
}

/// One in-flight [`AsyncBridge`] call: the input plus the slot to answer on.
pub struct Exchange<In, Out> {
    input: In,
    reply: Reply<Out>,
}

/// Movable, single-use handle that answers one [`AsyncBridge`] call.
///
/// Held as an `Arc<Signal>` so it is `Send + Sync` (when `Out: Send`) and can be
/// moved into a spawned task — e.g. the router's `run_local` bridge task — to be
/// answered after the draining system has returned.
pub struct Reply<Out> {
    signal: alloc::sync::Arc<Signal<CriticalSectionRawMutex, Out>>,
}

impl<In, Out: Send, const N: usize> AsyncBridge<In, Out, N> {
    pub const fn new() -> Self {
        Self {
            queue: Queue::new(),
        }
    }

    /// Initiator side: send `input` and await the response.
    ///
    /// On a full queue no reply is ever produced, so the rejected `input` is
    /// handed back in `Err` for the caller to do its own fallback (the client
    /// bails; the server writes a 503).
    pub async fn call(&self, input: In) -> Result<Out, In> {
        let signal = alloc::sync::Arc::new(Signal::new());
        let reply = Reply {
            signal: signal.clone(),
        };
        self.queue
            .send(Exchange { input, reply })
            .map_err(|ex| ex.input)?;
        Ok(signal.wait().await)
    }

    /// Responder side: await the next call (for an embassy driver loop).
    pub async fn recv(&self) -> Exchange<In, Out> {
        self.queue.recv().await
    }

    /// Responder side: take the next call if one is ready, without awaiting (for
    /// a per-frame Bevy drain system).
    pub fn try_recv(&self) -> Option<Exchange<In, Out>> {
        self.queue.try_recv()
    }
}

impl<In, Out, const N: usize> Default for AsyncBridge<In, Out, N>
where
    Out: Send,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<In, Out> Exchange<In, Out> {
    /// Borrow the input without consuming the exchange.
    pub fn input(&self) -> &In {
        &self.input
    }

    /// Split into the owned input and its [`Reply`] handle, so an async ECS
    /// dispatch can move the reply into a spawned task and answer it later.
    pub fn split(self) -> (In, Reply<Out>) {
        (self.input, self.reply)
    }
}

impl<Out> Reply<Out> {
    /// Answer the call. Consumes `self`, so a call is answered at most once.
    pub fn send(self, out: Out) {
        self.signal.signal(out);
    }
}

/// One pooled embassy task that drives any `'static` future to completion.
///
/// Type-erasing the future behind a `Box` (we have `alloc`) lets a single task
/// definition serve every async driver — an embassy `#[task]` can't be generic,
/// and a per-driver task would need a fixed-size pool slot big enough for its
/// largest future. Here each slot is just a box pointer, so even the big
/// `App`-holding tick future lives on the heap.
///
/// Slots are cheap (a box pointer + task header), and beside the long-lived
/// drivers the pool now also carries short-lived tasks — one per live socket
/// connection and one per in-flight `sleep_driver` timer — so 16 leaves
/// comfortable headroom.
#[embassy_executor::task(pool_size = 16)]
async fn run_boxed(fut: Pin<Box<dyn Future<Output = ()> + 'static>>) {
    fut.await;
}

/// Spawn `fut` as a long-lived driver on the executor.
///
/// The future may be `!Send` (the thread-mode [`Spawner`] runs it on the
/// executor's own thread) and lives on the heap, costing one allocation at
/// startup — long-lived, so no fragmentation.
pub fn spawn_driver(spawner: Spawner, fut: impl Future<Output = ()> + 'static) {
    let token = run_boxed(Box::pin(fut)).expect("driver task pool exhausted");
    spawner.spawn(token);
}

// ---------------------------------------------------------------------------
// Toolkit: the two reusable glue shapes over the primitives above.
//
// Every bridged capability is one of two shapes (see the module docs and
// `async_bridge_ergonomics.md`):
//
//   1. *ECS asks, silicon answers* (request/reply). An [`AsyncBridge`] whose
//      embassy side runs `loop { recv; reply.send(worker(job).await) }`. Only
//      `worker` differs. [`spawn_worker`] stands that loop up from a `worker`.
//   2. *silicon notifies, ECS reacts* (notification/stream). A [`Queue`] an
//      embassy producer pushes into, drained per-frame on the ECS side and
//      turned into `commands.trigger`s. Only the producer differs.
//      [`drain_to_observers`] is that drain.
//
// The server's *embassy initiates, ECS answers* case is shape 1 run the other
// way round: the responder is the ECS, so its drain ([`drain_to_async`]) is a
// per-frame system rather than an embassy loop. The `worker` there is an async
// world dispatch, so it runs on the bevy pool via `run_local`.
// ---------------------------------------------------------------------------

/// Shape 1, embassy side: run the request/reply loop for `bridge`, answering
/// each call with `worker(input).await`.
///
/// This is the whole embassy responder for an [`AsyncBridge`] — the loop is
/// always `recv; reply.send(worker(job).await)`; a capability supplies only the
/// `worker`. The `worker` body stays irreducibly peripheral-specific (its
/// buffers, sockets and setup); this only removes the boilerplate loop around
/// it.
///
/// `bridge` is a `&'static` so it can be a `static` shared with the initiator;
/// the returned future is `'static` and ready for [`spawn_driver`]. One job is
/// serviced at a time, matching the existing single-socket drivers.
pub async fn run_worker<In, Out, const N: usize, Fut>(
    bridge: &'static AsyncBridge<In, Out, N>,
    worker: impl Fn(In) -> Fut,
) where
    Out: Send,
    Fut: Future<Output = Out>,
{
    loop {
        let (input, reply) = bridge.recv().await.split();
        reply.send(worker(input).await);
    }
}

/// Spawn [`run_worker`] for `bridge` as a long-lived embassy driver.
///
/// Convenience over `spawn_driver(spawner, run_worker(bridge, worker))` so a
/// request/reply capability's embassy side is one line: hand it the `static`
/// bridge and an `async fn(In) -> Out`.
pub fn spawn_worker<In, Out, const N: usize, Fut>(
    spawner: Spawner,
    bridge: &'static AsyncBridge<In, Out, N>,
    worker: impl Fn(In) -> Fut + 'static,
) where
    In: 'static,
    Out: Send + 'static,
    Fut: Future<Output = Out> + 'static,
{
    spawn_driver(spawner, run_worker(bridge, worker));
}

/// Shape 2, ECS side: drain every pending item from `queue` and `trigger` it as
/// an observer [`Event`](beet::prelude::Event) on the world.
///
/// This is the generic notification drain: an embassy producer pushes events
/// into a `static` [`Queue<T>`] (a GPIO ISR, received UDP/mDNS packets, ...) and
/// the world reacts via observers, with the producer the only thing that
/// differs. Call it from a per-frame Bevy system over your event's `static`
/// queue:
///
/// ```ignore
/// static PACKETS: Queue<UdpPacket, 8> = Queue::new();
/// fn drain_packets(commands: Commands) { drain_to_observers(&PACKETS, commands); }
/// ```
///
/// Each drained `T` is delivered with `commands.trigger`, so any
/// `On<T>` observer in the world fires. Lossless up to the queue
/// bound; items the producer dropped on a full queue never arrive (drop-newest).
#[cfg(feature = "action")]
pub fn drain_to_observers<T, const N: usize>(
    queue: &'static Queue<T, N>,
    mut commands: beet::prelude::Commands,
) where
    for<'a> T: beet::prelude::Event<Trigger<'a>: Default> + 'static,
{
    while let Some(item) = queue.try_recv() {
        commands.trigger(item);
    }
}

/// Shape 1, ECS side: drain every pending request from `bridge` and answer each
/// by running `worker` as a `!Send` async world task, replying with its output.
///
/// The mirror of [`run_worker`] for the case where the *ECS* is the responder
/// (the server: embassy's accept loop initiates with [`AsyncBridge::call`], the
/// world produces the [`Response`](beet::prelude::Response)). The `worker` runs
/// on the bevy task pool via [`run_local`](beet::prelude::AsyncCommands::run_local)
/// — the only executor that can `.await` `&mut World` — and the [`Reply`] is
/// moved into that task to be answered once it resolves, possibly several frames
/// later. A capability supplies only the `worker: async fn(AsyncWorld, In) -> Out`.
///
/// Call it from a per-frame Bevy system:
///
/// ```ignore
/// fn drain(commands: AsyncCommands) {
///     drain_to_async(&BRIDGE, commands, |world, input| async move { /* ... */ });
/// }
/// ```
#[cfg(feature = "action")]
pub fn drain_to_async<In, Out, const N: usize, Fut>(
    bridge: &'static AsyncBridge<In, Out, N>,
    commands: beet::prelude::AsyncCommands,
    worker: impl Fn(beet::prelude::AsyncWorld, In) -> Fut + Clone + 'static,
) where
    In: 'static,
    Out: Send + 'static,
    Fut: Future<Output = Out> + 'static,
{
    while let Some((input, reply)) = bridge.try_recv().map(Exchange::split) {
        let worker = worker.clone();
        commands.run_local(async move |world: beet::prelude::AsyncWorld| {
            reply.send(worker(world, input).await);
        });
    }
}
