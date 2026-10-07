//! beet's [`HttpServer`] component on this no_std target, served over Wi-Fi.
//!
//! The standard beet pattern applies unchanged: spawn
//! `(HttpServer::new(port), CallOnReady::on_spawn(), children![dispatch])`,
//! where the dispatch host is the server's child (a bare
//! [`exchange_ext::handler`] or a [`Router::with_defaults`] over its routes).
//! Here the server is backed by the ESP32 embassy + `embassy-net` TCP stack
//! instead of `async-io`/hyper.
//!
//! The backend is installed once via [`HttpServer::set_backend`].
//! [`CallOnReady::on_spawn`] calls the server entity's `RunningSet`, which starts
//! the server's facet: it runs [`start_esp_server`] on the async layer with an
//! [`AsyncEntity`], which reads the port off the component, grabs the embassy
//! [`Stack`]/[`Spawner`] published by [`start_wifi`](super::start_wifi), spawns
//! an embassy accept loop, then holds the facet open until its shutdown signal.
//!
//! ## The embassy↔ECS split
//!
//! TCP accept/read/write runs on **embassy** (where the net stack lives, for
//! immediate wakeups). Dispatch needs **the bevy task pool** (it awaits
//! `&mut World`, which only exists inside the `BeetAsyncSyncPoint` window). The
//! two are bridged by a [`static`](SERVER_BRIDGE) [`AsyncBridge`]: the accept
//! loop hands each parsed [`Request`] across, [`drain_server_requests`]
//! dispatches it through `entity.exchange_child` in the sync window, and the
//! [`Response`] comes back on the reply slot. Awaiting the dispatch directly on
//! an embassy task would live-lock (see `async_utils`), hence the bridge.

use crate::esp32_utils::async_bridge::AsyncBridge;
use crate::esp32_utils::async_bridge::drain_to_async;
use crate::esp32_utils::async_bridge::spawn_driver;
use alloc::sync::Arc;
use beet::prelude::*;
use embassy_executor::Spawner;
use embassy_futures::select::select;
use embassy_net::Stack;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Duration;
use embedded_io_async::Write as _;

/// Accepted requests handed from a [`server_loop`] to the ECS, each carrying the
/// server [`Entity`] it arrived on (whose dispatch child answers it) and a reply
/// slot. Drained by [`drain_server_requests`].
static SERVER_BRIDGE: AsyncBridge<(Entity, Request), Response, 4> = AsyncBridge::new();

/// Plugin: install the ESP32 server backend and the drain system.
///
/// A plain `fn(&mut App)` *is* a Bevy [`Plugin`], so this is added with
/// `app.add_plugins(http_server_plugin)` (see [`WifiPlugin::build`](super::WifiPlugin),
/// under the `action` feature). The install must precede any server start so
/// the facet finds a backend; a second install would already own it, so just
/// warn.
pub(crate) fn http_server_plugin(app: &mut App) {
    if HttpServer::set_backend(start_esp_server).is_err() {
        warn!("an HTTP server backend was already installed");
    }
    app.add_systems(Update, drain_server_requests);
}

/// Stops a server's embassy accept loop, shared between the backend future (the
/// beet side) and the loop's task. A [`Signal`] is executor-agnostic, so the beet
/// side can raise it and the embassy task wakes on it.
type StopSignal = Arc<Signal<CriticalSectionRawMutex, ()>>;

/// Raises its [`StopSignal`] when dropped, so the accept loop ends however the
/// facet does: returning on the shutdown signal, or a despawn dropping the
/// backend future outright.
struct StopOnDrop(StopSignal);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.signal(());
    }
}

/// beet's server backend hook (see [`HttpServer::set_backend`]): run on the async
/// layer with an [`AsyncEntity`], mirroring `HttpServer::start_mini`.
///
/// Reads the port off the [`HttpServer`] component, waits for the embassy
/// [`Stack`]/[`Spawner`] (published by [`start_wifi`](super::start_wifi)) to
/// exist, since the facet can start before Wi-Fi is up, then spawns the accept
/// loop (plus the mDNS responder when the entity carries an `MDns`).
/// The future then holds the server open until `shutdown` resolves; ending it
/// stops the embassy task, closing the listening socket and the responder.
fn start_esp_server(
    entity: AsyncEntity,
    shutdown: OnceValueRx<()>,
) -> LocalBoxedFuture<'static, Result> {
    Box::pin(async move {
        let id = entity.id();
        let port = entity
            .get::<HttpServer, u16>(|server| server.port.unwrap_or(DEFAULT_HTTP_PORT))
            .await?;

        // If the server entity also carries an `MDns` component, grab its hostname
        // so the accept-loop spawn can also start the mDNS responder advertising
        // `hostname.local`. `entity.get` errors when the component is absent, so a
        // plain `HttpServer` (no mDNS) just yields `None`.
        #[cfg(feature = "mdns")]
        let mdns_hostname: Option<&'static str> = entity
            .get::<super::mdns::MDns, &'static str>(|mdns| mdns.hostname)
            .await
            .ok();

        let stop: StopSignal = Arc::new(Signal::new());
        let _stop_on_drop = StopOnDrop(stop.clone());

        // The facet may start before `start_wifi` has published the Stack. Each
        // `world().with(..)` round-trips a full sync window, so this naturally
        // polls one frame at a time until the Stack appears, then spawns the
        // accept loop from inside the window (we are on the embassy thread there).
        loop {
            let stop = stop.clone();
            let started = entity
                .world()
                .with(move |world: &mut World| {
                    let Some(stack) =
                        world.get_non_send::<Stack<'static>>().copied()
                    else {
                        return false;
                    };
                    let spawner = *world.non_send::<Spawner>();
                    let serve = async move {
                        // One mDNS task per server entity that asked for it; it
                        // owns the multicast socket and serves both responder and
                        // resolver, living exactly as long as the accept loop.
                        #[cfg(feature = "mdns")]
                        if let Some(hostname) = mdns_hostname {
                            embassy_futures::join::join(
                                server_loop(stack, port, id),
                                super::mdns::mdns_task(stack, hostname),
                            )
                            .await;
                            return;
                        }
                        server_loop(stack, port, id).await
                    };
                    spawn_driver(spawner, async move {
                        select(serve, stop.wait()).await;
                        info!("HTTP server on :{} stopped", port);
                    });
                    true
                })
                .await;
            if started {
                break;
            }
        }
        shutdown.wait().await;
        Ok(())
    })
}

/// Accept connections forever; parse each into a beet [`Request`], hand it to the
/// ECS for a [`Response`] via [`SERVER_BRIDGE`], and serialise the reply back to
/// the socket. Runs as an embassy task.
async fn server_loop(stack: Stack<'static>, port: u16, entity: Entity) {
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("HTTP server: http://{}:{}", cfg.address.address(), port);
    }

    loop {
        let mut rx = [0u8; 1536];
        let mut tx = [0u8; 1536];
        let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
        socket.set_timeout(Some(Duration::from_secs(10)));

        if let Err(e) = socket.accept(port).await {
            warn!("accept failed: {:?}", e);
            continue;
        }

        let raw = read_http(&mut socket).await;
        let request = match http_ext::parse_http_request(&raw) {
            Ok(request) => request,
            Err(e) => {
                warn!("malformed request: {:?}", e);
                let _ = socket
                    .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                let _ = socket.flush().await;
                socket.close();
                continue;
            }
        };

        let response = match SERVER_BRIDGE.call((entity, request)).await {
            Ok(response) => response,
            Err(_) => {
                warn!("server queue full; dropping request on :{}", port);
                let _ = socket
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                    .await;
                let _ = socket.flush().await;
                socket.close();
                continue;
            }
        };

        let bytes = match http_ext::serialize_http_response(response).await {
            Ok(bytes) => bytes,
            Err(e) => {
                warn!("serialise failed: {:?}", e);
                socket.close();
                continue;
            }
        };
        if let Err(e) = socket.write_all(&bytes).await {
            warn!("write failed: {:?}", e);
        }
        if let Err(e) = socket.flush().await {
            warn!("flush failed: {:?}", e);
        }
        socket.close();
    }
}

/// Drain queued requests from [`SERVER_BRIDGE`] and dispatch each through its
/// server entity's dispatch child inside the bridge sync-point.
///
/// One path for every request: the server parks on its own `RunningSet` action,
/// so `exchange_child` hands each request down to the first child serving
/// `Request -> Response`, a bare [`exchange_ext::handler`] or a [`Router`]. The
/// dispatch spans the async action layer, so a request may take a few frames,
/// the same `BeetAsyncSyncPoint` machinery the behavior-tree example drives.
///
/// The drain/dispatch/reply boilerplate is the request/reply toolkit's
/// [`drain_to_async`] (the ECS-responder mirror of `run_worker`); this supplies
/// only the `worker`: route the `(entity, request)` through `exchange_child`.
fn drain_server_requests(commands: AsyncCommands) {
    drain_to_async(
        &SERVER_BRIDGE,
        commands,
        |world: AsyncWorld, (target, request)| async move {
            world.entity(target).exchange_child(request).await
        },
    );
}

/// Read a full HTTP message off the socket: headers, plus the body if a
/// `Content-Length` says there is one. Stops at EOF or once the body is in.
async fn read_http(socket: &mut TcpSocket<'_>) -> Vec<u8> {
    const CAP: usize = 8192;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match socket.read(&mut chunk).await {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(header_end) = http_ext::find_header_end(&buf) {
                    let content_length =
                        http_ext::parse_content_length(&buf[..header_end]);
                    if buf.len() >= header_end + content_length {
                        break;
                    }
                }
                if buf.len() >= CAP {
                    break;
                }
            }
            Err(e) => {
                warn!("read failed: {:?}", e);
                break;
            }
        }
    }
    buf
}
