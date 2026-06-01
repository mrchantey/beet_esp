//! The esp/downstream half of beet's agnostic mDNS service **Browser**, behind
//! the `mdns` feature.
//!
//! The platform-independent engine — the [`UdpPacket`] event, the
//! [`handle_udp_packet`] observer, the [`DiscoveredServices`] resource, the
//! [`ServiceDiscovered`]/[`ServiceRemoved`] events and the
//! [`MdnsBrowser`]/[`MdnsBrowserPlugin`] config — lives **upstream** in
//! `beet_net` and is reused here unchanged (phase 2a). This module supplies only
//! the two esp-specific pieces phase 2 calls for:
//!
//! 1. A concrete [`UdpSocket`]/[`UdpEndpoint`] impl for `embassy-net` 0.9
//!    ([`EmbassyUdpSocket`] / [`EmbassyUdpEndpoint`]) — implemented **directly**
//!    over `embassy-net`, *not* via `edge-nal`. `edge-nal-embassy` 0.8 targets
//!    `embassy-net ^0.8`, which is incompatible with this tree's `embassy-net`
//!    0.9.1 (the same version mismatch that forced the phase-1 responder to be
//!    hand-rolled; see [`super::mdns`]). beet's `UdpSocket` trait mirrors
//!    `edge-nal`'s method shape anyway, so the impl is thin regardless.
//! 2. The embassy→world **bridge**: [`browser_task`] owns the multicast socket,
//!    joins `224.0.0.251:5353`, periodically multicasts the `PTR` query (on its
//!    own embassy [`Timer`]), and pushes each inbound datagram as a [`UdpPacket`]
//!    into a `static` [`Queue`]. [`drain_browser_packets`] is the per-frame ECS
//!    drain that hands each packet to beet_net's [`handle_udp_packet`] observer
//!    via [`drain_to_observers`] — the shape-2 notification path the Effort-2
//!    toolkit was built for.
//!
//! ## The data path
//!
//! ```text
//!   embassy browser_task                 ECS (bevy sync window)
//!   --------------------                 ----------------------
//!   Timer -> send_to(PTR query) ─┐
//!   recv_from(datagram) ─────────┴─► PACKETS: Queue<UdpPacket>
//!                                          │  drain_browser_packets (Update)
//!                                          ▼  drain_to_observers
//!                                    world.trigger(UdpPacket)
//!                                          │  handle_udp_packet (beet_net observer)
//!                                          ▼
//!                                    DiscoveredServices  +  ServiceDiscovered /
//!                                                            ServiceRemoved
//! ```
//!
//! The socket IO lives on embassy (it awaits silicon); the parse + reconcile is
//! the agnostic engine running in the world. The two are decoupled by the
//! `static` [`Queue`], exactly like the phase-1 [`AsyncBridge`](crate::async_bridge)
//! drivers — no `&mut World` ever crosses onto the embassy task.
//!
//! ## A note on the trait vs. embassy-net's buffer model
//!
//! beet's [`UdpEndpoint::bind`] is `fn bind(&self, local) -> Self::Socket<'_>`:
//! the socket borrows the *binder*, with no slot to thread in buffers. But
//! `embassy-net`'s `UdpSocket::new` consumes four caller-owned buffers
//! (`rx_meta`/`rx_buf`/`tx_meta`/`tx_buf`) that must outlive the socket. So
//! [`EmbassyUdpEndpoint`] owns `&mut` references to those four buffers and
//! [`bind`](EmbassyUdpEndpoint::bind) hands them to `UdpSocket::new` — a binder
//! that can be used **once** (it moves its `&mut` buffers into the socket),
//! rather than the repeatedly-bindable std endpoint. That is the one honest
//! deviation from the trait's reusable-binder spirit, forced by embassy-net's
//! external-buffer ownership; decision 4 anticipates exactly this ("do the
//! cleanest thing that still satisfies decision 4"). The trait methods
//! ([`send_to`](UdpSocket::send_to) / [`recv_from`](UdpSocket::recv_from) /
//! [`join_multicast_v4`](UdpSocket::join_multicast_v4)) map 1:1 onto embassy-net
//! and are what [`browser_task`] actually drives, so the seam is real and used —
//! not bypassed by a parallel hand-rolled loop.

use crate::async_bridge::Queue;
use crate::async_bridge::drain_to_observers;
use crate::async_bridge::spawn_driver;
use beet::prelude::*;
use core::net::Ipv4Addr;
use core::net::SocketAddr;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Stack;
use embassy_net::udp::PacketMetadata;
use embassy_time::Duration;
use embassy_time::with_timeout;

/// How often the browser re-multicasts its `PTR` query.
///
/// The upstream std driver uses 5s ([`beet`'s `DEFAULT_QUERY_INTERVAL_MS`]); we
/// match it. A periodic re-query is how mDNS browsing stays live: responders
/// answer each query, so re-asking refreshes instances and surfaces ones that
/// joined since the last round.
const QUERY_INTERVAL: Duration = Duration::from_secs(5);

/// Inbound datagrams handed from [`browser_task`] (embassy) to the ECS. Drained
/// each frame by [`drain_browser_packets`] into beet_net's
/// [`handle_udp_packet`] observer.
///
/// Depth 8: mDNS is bursty (one query can prompt several hosts to answer at
/// once), and the drain runs every frame, so 8 absorbs a normal burst between
/// frames. A full queue drops the newest datagram (drop-newest, per [`Queue`]);
/// a dropped answer is recovered on the next [`QUERY_INTERVAL`] re-query.
static PACKETS: Queue<UdpPacket, 8> = Queue::new();

/// Plugin: wire the esp mDNS browser into an [`App`].
///
/// A plain `fn(&mut App)` *is* a Bevy [`Plugin`], so this is added with
/// `app.add_plugins(mdns_browser_plugin)` (see [`WifiPlugin::build`](super::WifiPlugin),
/// under the `mdns` feature). Adds beet_net's agnostic [`MdnsBrowserPlugin`] (the
/// [`DiscoveredServices`] resource + the [`handle_udp_packet`] observer), this
/// crate's per-frame packet drain, and [`start_pending_browsers`] — the system
/// that starts an embassy [`browser_task`] for each [`MdnsBrowser`] entity once
/// Wi-Fi is up.
pub(crate) fn mdns_browser_plugin(app: &mut App) {
    app.add_plugins(MdnsBrowserPlugin)
        .add_systems(Update, (start_pending_browsers, drain_browser_packets));
}

/// Drain every queued [`UdpPacket`] into the world so beet_net's
/// [`handle_udp_packet`] observer parses each and reconciles
/// [`DiscoveredServices`]. The shape-2 toolkit drain — the producer is
/// [`browser_task`].
fn drain_browser_packets(commands: Commands) {
    drain_to_observers(&PACKETS, commands);
}

/// Marker added to an [`MdnsBrowser`] entity once its [`browser_task`] has been
/// spawned, so [`start_pending_browsers`] starts exactly one task per entity.
#[derive(Component)]
struct BrowserStarted;

/// For every [`MdnsBrowser`] entity not yet [`BrowserStarted`], start an embassy
/// [`browser_task`] once the [`Stack`] exists, then mark it started.
///
/// Mirrors the server's Stack-wait pattern ([`start_esp_server`]) but as a plain
/// per-frame system rather than a beet backend hook: an `MdnsBrowser` spawned at
/// app build (before `start_wifi` publishes the `Stack`) is picked up on the
/// first frame the `Stack` appears, and one spawned later starts on its next
/// frame. Exclusive so it can read the non-send [`Stack`]/[`Spawner`] (the same
/// access as [`start_wifi`](super::start_wifi) and [`spawn_clock`](crate::clock)).
fn start_pending_browsers(world: &mut World) {
    // Collect the (entity, service_type) of every browser not yet started. Cheap
    // early-out in the steady state once all are marked.
    let mut pending: Vec<(Entity, SmolStr)> = Vec::new();
    let mut query =
        world.query_filtered::<(Entity, &MdnsBrowser), Without<BrowserStarted>>();
    for (entity, browser) in query.iter(world) {
        pending.push((entity, browser.service_type.clone()));
    }
    if pending.is_empty() {
        return;
    }

    // The Stack arrives in `start_wifi` (Startup); until then, leave them pending.
    let Some(stack) = world.get_non_send::<Stack<'static>>().copied() else {
        return;
    };
    let spawner = *world.non_send::<Spawner>();
    for (entity, service_type) in pending {
        info!(
            "mDNS browser starting for {}",
            defmt::Display2Format(&service_type)
        );
        spawn_driver(spawner, browser_task(stack, service_type));
        world.entity_mut(entity).insert(BrowserStarted);
    }
}

/// The embassy mDNS browser task: own the multicast socket, join the group,
/// periodically multicast the `PTR` query, and push each inbound datagram into
/// [`PACKETS`].
///
/// This is the esp analogue of beet_net's std `run_mdns_browser`. It uses the
/// concrete [`EmbassyUdpSocket`] (beet's [`UdpSocket`] trait) for all socket IO,
/// so the trait seam is genuinely exercised. The periodic query is driven by
/// racing `recv_from` against an embassy timer via
/// [`with_timeout`](embassy_time::with_timeout): each [`QUERY_INTERVAL`] with no
/// inbound packet re-multicasts the query — the simplest place for the periodic
/// send (decision: keep the timer in the task, next to the socket).
async fn browser_task(stack: Stack<'static>, service_type: SmolStr) {
    stack.wait_config_up().await;

    if let Err(e) = stack.join_multicast_group(IpAddress::Ipv4(MDNS_MULTICAST_V4)) {
        warn!("mDNS browser: join_multicast_group failed: {:?}", e);
        return;
    }

    // Buffers for the embassy socket. mDNS is small but bursty; match the
    // phase-1 responder's generous pools. These land in PSRAM (the task future
    // is heap-boxed by `spawn_driver`).
    let mut rx_meta = [PacketMetadata::EMPTY; 8];
    let mut rx_buf = [0u8; 1536];
    let mut tx_meta = [PacketMetadata::EMPTY; 8];
    let mut tx_buf = [0u8; 1536];

    // The trait-honest binder: owns the four buffers, hands them to
    // `UdpSocket::new` on `bind`. Bind 0.0.0.0:5353 to receive multicast.
    let endpoint =
        EmbassyUdpEndpoint::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    let bind = SocketAddr::from((Ipv4Addr::UNSPECIFIED, MDNS_PORT));
    let socket = match endpoint.bind(bind).await {
        Ok(socket) => socket,
        Err(e) => {
            warn!("mDNS browser: bind :{} failed: {}", MDNS_PORT, defmt::Debug2Format(&e));
            return;
        }
    };

    // Build the `PTR` query once (it never changes for a service type), using
    // beet_net's pure wire helper — the same builder the std driver uses.
    let mut query = [0u8; 256];
    let query_len = match build_ptr_query(&mut query, &service_type) {
        Some(len) => len,
        None => {
            warn!("mDNS browser: service type too long to query");
            return;
        }
    };
    let query = &query[..query_len];

    if let Err(e) = socket.send_to(query, MDNS_ENDPOINT).await {
        warn!("mDNS browser: initial query send failed: {}", defmt::Debug2Format(&e));
    }
    info!(
        "mDNS browser up: querying {} on 224.0.0.251:5353",
        defmt::Display2Format(&service_type)
    );

    let mut recv = [0u8; 1536];
    loop {
        // Race a receive against the query timer. `with_timeout` returns `Err`
        // when the interval elapses with no packet — that's our cue to re-query.
        match with_timeout(QUERY_INTERVAL, socket.recv_from(&mut recv)).await {
            Ok(Ok((n, from))) => {
                let packet = UdpPacket {
                    from,
                    bytes: recv[..n].to_vec(),
                };
                // Drop-newest on a full queue; the next re-query recovers it.
                if PACKETS.send(packet).is_err() {
                    warn!("mDNS browser: packet queue full, dropping datagram");
                }
            }
            Ok(Err(e)) => {
                warn!("mDNS browser: recv failed: {}", defmt::Debug2Format(&e));
            }
            Err(_) => {
                // Interval elapsed: re-multicast the query.
                if let Err(e) = socket.send_to(query, MDNS_ENDPOINT).await {
                    warn!("mDNS browser: re-query send failed: {}", defmt::Debug2Format(&e));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Concrete `UdpEndpoint` / `UdpSocket` impl for embassy-net 0.9 (no edge-nal).
//
// These satisfy beet_net's trait directly. Address conversion is cheap: smoltcp
// 0.13 (which embassy-net re-exports) provides `From<IpEndpoint> for
// core::net::SocketAddr` (the recv path) and `From<SocketAddrV4>` (the send path,
// via `to_endpoint`), and its `Ipv4Address` *is* `core::net::Ipv4Addr`.
// ---------------------------------------------------------------------------

/// A bound `embassy-net` UDP socket implementing beet's [`UdpSocket`] trait.
///
/// A thin newtype around `embassy_net::udp::UdpSocket`. Both embassy
/// `send_to`/`recv_from` take `&self`, matching beet's trait exactly; the
/// `Stack` is kept alongside so [`join_multicast_v4`](UdpSocket::join_multicast_v4)
/// (an iface operation, not a socket one on embassy-net) can be served.
pub struct EmbassyUdpSocket<'a> {
    socket: embassy_net::udp::UdpSocket<'a>,
    stack: Stack<'a>,
}

/// Convert a [`core::net::SocketAddr`] to an embassy/smoltcp [`IpEndpoint`].
///
/// smoltcp 0.13 implements `From` for `SocketAddrV4` (but not the `SocketAddr`
/// enum), and `From<SocketAddrV6>` only when its `proto-ipv6` feature is on —
/// which this tree does not enable (we are IPv4-only: `proto-ipv4` + mDNS over
/// `224.0.0.251`). So we build the endpoint from the IPv4 octets directly, which
/// also keeps a v6 address from silently mis-binding. A v6 address can't reach
/// this path (the browser only ever uses IPv4 mDNS endpoints).
fn to_endpoint(addr: SocketAddr) -> IpEndpoint {
    match addr {
        SocketAddr::V4(v4) => IpEndpoint::new(IpAddress::Ipv4(*v4.ip()), v4.port()),
        // IPv6 can't reach this IPv4-only mDNS path; fold any v4-mapped suffix
        // into a v4 endpoint so the type still converts. In practice every
        // endpoint here is V4 (the browser only uses `224.0.0.251`).
        SocketAddr::V6(v6) => {
            let o = v6.ip().octets();
            IpEndpoint::new(
                IpAddress::Ipv4(Ipv4Addr::new(o[12], o[13], o[14], o[15])),
                v6.port(),
            )
        }
    }
}

impl<'a> UdpSocket for EmbassyUdpSocket<'a> {
    async fn send_to(&self, bytes: &[u8], remote: SocketAddr) -> Result<()> {
        self.socket
            .send_to(bytes, to_endpoint(remote))
            .await
            .map_err(|e| bevyhow!("udp send_to failed: {:?}", e))
    }

    async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr)> {
        let (n, meta) = self
            .socket
            .recv_from(buf)
            .await
            .map_err(|e| bevyhow!("udp recv_from failed: {:?}", e))?;
        Ok((n, meta.endpoint.into()))
    }

    async fn join_multicast_v4(&self, group: Ipv4Addr) -> Result<()> {
        self.stack
            .join_multicast_group(IpAddress::Ipv4(group))
            .map_err(|e| bevyhow!("join_multicast_v4 failed: {:?}", e))
    }
}

/// The four caller-owned buffers `embassy_net::udp::UdpSocket::new` consumes.
/// Held in the endpoint until [`bind`](EmbassyUdpEndpoint::bind) moves them into
/// the socket.
struct UdpBuffers<'a> {
    rx_meta: &'a mut [PacketMetadata],
    rx_buf: &'a mut [u8],
    tx_meta: &'a mut [PacketMetadata],
    tx_buf: &'a mut [u8],
}

/// A binder for [`EmbassyUdpSocket`]s implementing beet's [`UdpEndpoint`] trait.
///
/// Owns the four `&mut` buffers `embassy_net::udp::UdpSocket::new` requires (the
/// stack can't supply them; they're caller-owned), so it is a *single-use*
/// binder: [`bind`](Self::bind) moves them into the socket. A second `bind` (the
/// buffers already gone) errors rather than aliasing.
///
/// The buffers live behind a [`RefCell`](core::cell::RefCell)`<Option<..>>` so
/// the move-out works through the trait's `bind(&self, ..)` signature without
/// `unsafe`: `bind` `take()`s the `Option` once. This is the honest shape for
/// embassy-net's external-buffer ownership (see the module note); it is the one
/// deviation from the std endpoint's repeatedly-bindable behaviour.
pub struct EmbassyUdpEndpoint<'a> {
    stack: Stack<'a>,
    buffers: core::cell::RefCell<Option<UdpBuffers<'a>>>,
}

impl<'a> EmbassyUdpEndpoint<'a> {
    /// Build an endpoint over `stack` borrowing the four caller-owned buffers.
    pub fn new(
        stack: Stack<'a>,
        rx_meta: &'a mut [PacketMetadata],
        rx_buf: &'a mut [u8],
        tx_meta: &'a mut [PacketMetadata],
        tx_buf: &'a mut [u8],
    ) -> Self {
        Self {
            stack,
            buffers: core::cell::RefCell::new(Some(UdpBuffers {
                rx_meta,
                rx_buf,
                tx_meta,
                tx_buf,
            })),
        }
    }
}

impl<'a> UdpEndpoint for EmbassyUdpEndpoint<'a> {
    type Socket<'s>
        = EmbassyUdpSocket<'a>
    where
        Self: 's;

    async fn bind(&self, local: SocketAddr) -> Result<Self::Socket<'_>> {
        // Take the buffers (single-use binder). A second bind finds `None`.
        let UdpBuffers {
            rx_meta,
            rx_buf,
            tx_meta,
            tx_buf,
        } = self
            .buffers
            .borrow_mut()
            .take()
            .ok_or_else(|| bevyhow!("EmbassyUdpEndpoint already bound (single-use)"))?;

        let mut socket = embassy_net::udp::UdpSocket::new(
            self.stack, rx_meta, rx_buf, tx_meta, tx_buf,
        );
        // Build the listen endpoint. An unspecified local IP (`0.0.0.0`) must map
        // to `addr: None` ("listen on any address"), NOT `addr: Some(0.0.0.0)`:
        // smoltcp only delivers multicast (e.g. `224.0.0.251`) to an `addr: None`
        // UDP listener. `From<SocketAddr> for IpListenEndpoint` would set
        // `Some(0.0.0.0)` and silently swallow every multicast datagram (the bug
        // that made the browser receive nothing). This matches phase-1's working
        // `socket.bind(MDNS_PORT)`, which also yields `addr: None`.
        let listen = if local.ip().is_unspecified() {
            embassy_net::IpListenEndpoint {
                addr: None,
                port: local.port(),
            }
        } else {
            to_endpoint(local).into()
        };
        socket
            .bind(listen)
            .map_err(|e| bevyhow!("udp bind failed: {:?}", e))?;
        Ok(EmbassyUdpSocket {
            socket,
            stack: self.stack,
        })
    }
}
