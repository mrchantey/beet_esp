//! The ECS↔async bridge: typed sync→async pipes plus a type-erased spawner for
//! long-lived embassy drivers.
//!
//! Every async driver shares one shape: a task that pulls a message from a
//! sync→async pipe, `.await`s on hardware/IO, and optionally pushes results back
//! through another pipe. The LED is the simplest case (pipe = [`Latest`],
//! message = `Color`, await = `Ws2812::write`, no result); a Wi-Fi worker would
//! use a [`Queue`] of requests and a [`Queue`] of responses. Same pipe types,
//! same [`spawn_driver`].

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

/// One pooled embassy task that drives any `'static` future to completion.
///
/// Type-erasing the future behind a `Box` (we have `alloc`) lets a single task
/// definition serve every async driver — an embassy `#[task]` can't be generic,
/// and a per-driver task would need a fixed-size pool slot big enough for its
/// largest future. Here each slot is just a box pointer, so even the big
/// `App`-holding tick future lives on the heap.
#[embassy_executor::task(pool_size = 8)]
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
