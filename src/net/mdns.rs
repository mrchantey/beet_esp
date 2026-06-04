//! Minimal mDNS (`.local` name service) over the Wi-Fi stack, behind the `mdns`
//! feature.
//!
//! mDNS is the DNS wire format (RFC 1035 records) carried over UDP multicast on
//! `224.0.0.251:5353`. It is not HTTP and does not fit beet's `Request`/`Response`
//! shape, so it lives here as a self-contained module rather than going through
//! beet's transport hooks. Two roles are served, both by **one** embassy task that
//! owns the single multicast socket:
//!
//! - **Responder** — advertises us. We claim `hostname.local` and answer multicast
//!   `A` queries for it with our IPv4. This is what makes
//!   `curl http://hostname.local:8080` work from another machine on the LAN.
//! - **Resolver** — resolves one `.local` peer name to an IP for the HTTP client.
//!   The client hands a name across the [`RESOLVER`] bridge; this task multicasts a
//!   query and matches the `A` answer back.
//!
//! ## Why hand-rolled, not edge-mdns
//!
//! `edge-mdns` (the edge-net family responder) was the first choice, but its
//! embassy adapter `edge-nal-embassy` 0.8 targets `embassy-net ^0.8` while this
//! tree pins `embassy-net` 0.9.1, and `edge-mdns` 0.7 wants `embassy-sync ^0.7`
//! while we pin `embassy-sync` 0.8 (to match `esp-rtos`). Neither aligns. The DNS
//! wire format for a single `A` query/answer is small and self-contained, and
//! embassy-net 0.9 already exposes a clean `UdpSocket` plus
//! `Stack::join_multicast_group`, so a minimal hand-rolled responder/resolver
//! integrates with zero version juggling. See `agent/plans/mdns.md`.
//!
//! ## Socket sharing
//!
//! One task, one socket. A UDP socket bound to `:5353` and joined to the multicast
//! group receives *all* mDNS traffic — both queries aimed at us and answers to our
//! own queries — so a single recv loop must demultiplex regardless. Two sockets on
//! the same port would also fight under smoltcp. So the responder and resolver
//! share this socket: the recv loop answers inbound queries and, when a resolver
//! job is pending, also matches inbound answers; the resolver's outbound query is
//! sent from the same socket.
//!
//! ## Scope (phase 1)
//!
//! `A` records only (`hostname.local -> ipv4`). No `_http._tcp.local` service
//! record (see the module note below for the rationale). IPv4 multicast only.

use crate::esp32_utils::async_bridge::AsyncBridge;
use beet::prelude::*;
use defmt::info;
use defmt::warn;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Ipv4Address;
use embassy_net::Stack;
use embassy_net::udp::PacketMetadata;
use embassy_net::udp::UdpSocket;
use embassy_time::Duration;
use embassy_time::with_timeout;

/// The IPv4 mDNS multicast group, `224.0.0.251`.
const MDNS_GROUP: Ipv4Address = Ipv4Address::new(224, 0, 0, 251);
/// The mDNS UDP port, `5353`.
const MDNS_PORT: u16 = 5353;
/// `.local` suffix every mDNS name carries.
const LOCAL_SUFFIX: &str = ".local";
/// How long the resolver waits for an `A` answer before giving up.
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(2);

/// beet component: when present on an [`HttpServer`](beet::prelude::HttpServer)
/// entity, the server's accept loop also starts the mDNS responder advertising
/// `hostname.local` at the device's IPv4.
///
/// The `hostname` is the bare label, without the `.local` suffix (e.g.
/// `"beet-esp"` advertises `beet-esp.local`).
#[derive(Component, Clone, Debug)]
pub struct MDns {
    /// The bare hostname label to advertise, e.g. `"beet-esp"` for
    /// `beet-esp.local`.
    pub hostname: &'static str,
}

impl MDns {
    /// Advertise `hostname.local`.
    pub fn new(hostname: &'static str) -> Self {
        Self { hostname }
    }
}

/// Resolver bridge: the HTTP client hands a bare `.local`-stripped hostname across
/// and awaits the resolved [`Ipv4Address`] (or `None` on timeout / no answer). One
/// in flight at a time, matching the single-socket design.
///
/// Direction: the client driver ([`exchange`](super::http_client)) initiates with
/// [`call`](AsyncBridge::call); the mDNS task drains it and replies after sending a
/// query and matching the answer.
static RESOLVER: AsyncBridge<String, Option<Ipv4Address>, 2> = AsyncBridge::new();

/// Resolve a `.local` host to an IPv4 via the running mDNS task.
///
/// Called from the HTTP client's `exchange()` when it sees a `.local` authority.
/// `host` is the full name (with or without the trailing `.local`); the suffix is
/// stripped before querying. Returns `None` if the mDNS task is not running, the
/// queue is full, or no answer arrives before [`RESOLVE_TIMEOUT`].
pub(crate) async fn resolve(host: &str) -> Option<Ipv4Address> {
    let label = host.strip_suffix(LOCAL_SUFFIX).unwrap_or(host);
    match RESOLVER.call(String::from(label)).await {
        Ok(answer) => answer,
        // Queue full (the mDNS task is busy or absent): no reply will arrive.
        Err(_) => None,
    }
}

/// The single mDNS task: owns the multicast socket, answers inbound `A` queries
/// for `hostname.local` (responder), and services resolver jobs (resolver).
///
/// Spawned by the server accept loop when an [`MDns`] component is present. Joins
/// the multicast group, then loops selecting between an inbound packet and a
/// pending resolver job.
pub(crate) async fn mdns_task(stack: Stack<'static>, hostname: &'static str) {
    stack.wait_config_up().await;

    if let Err(e) = stack.join_multicast_group(IpAddress::Ipv4(MDNS_GROUP)) {
        warn!("mDNS: join_multicast_group failed: {:?}", e);
        return;
    }

    // Generous metadata/buffer pools: mDNS packets are small but bursty (a single
    // query can prompt several hosts to answer at once). These land in PSRAM.
    let mut rx_meta = [PacketMetadata::EMPTY; 8];
    let mut rx_buf = [0u8; 1536];
    let mut tx_meta = [PacketMetadata::EMPTY; 8];
    let mut tx_buf = [0u8; 1536];
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buf,
        &mut tx_meta,
        &mut tx_buf,
    );

    if let Err(e) = socket.bind(MDNS_PORT) {
        warn!("mDNS: bind :{} failed: {:?}", MDNS_PORT, e);
        return;
    }

    let ip = current_ipv4(stack);
    info!(
        "mDNS responder up: {}.local -> {:?}",
        hostname,
        ip.map(|a| a.octets())
    );

    let group = IpEndpoint::new(IpAddress::Ipv4(MDNS_GROUP), MDNS_PORT);
    let mut recv = [0u8; 1536];

    loop {
        // Either a resolver job is pending (send a query, then wait for its
        // answer with a timeout) or we idle on inbound packets answering queries.
        match RESOLVER.try_recv() {
            Some(exchange) => {
                let (label, reply) = exchange.split();
                let answer =
                    do_resolve(&socket, group, &label, &mut recv, hostname, stack).await;
                reply.send(answer);
            }
            None => {
                // No resolver work: serve inbound queries. Wake on either a packet
                // or (via the short timeout) to re-check for a resolver job.
                match with_timeout(
                    Duration::from_millis(250),
                    socket.recv_from(&mut recv),
                )
                .await
                {
                    Ok(Ok((n, meta))) => {
                        handle_inbound(
                            &socket, group, &recv[..n], meta.endpoint, hostname, stack,
                        )
                        .await;
                    }
                    Ok(Err(e)) => warn!("mDNS: recv failed: {:?}", e),
                    // Timed out: loop back to re-check the resolver queue.
                    Err(_) => {}
                }
            }
        }
    }
}

/// Resolver path: send a multicast `A` query for `label.local`, then read answers
/// until one matches (or the timeout elapses). While waiting, also answer any
/// inbound queries for *our* name so resolving never makes us unresponsive.
async fn do_resolve(
    socket: &UdpSocket<'_>,
    group: IpEndpoint,
    label: &str,
    recv: &mut [u8],
    hostname: &'static str,
    stack: Stack<'static>,
) -> Option<Ipv4Address> {
    let mut query = [0u8; 256];
    let len = match build_query(&mut query, label) {
        Some(len) => len,
        None => {
            warn!("mDNS: name too long to query");
            return None;
        }
    };
    if let Err(e) = socket.send_to(&query[..len], group).await {
        warn!("mDNS: query send failed: {:?}", e);
        return None;
    }

    let deadline = embassy_time::Instant::now() + RESOLVE_TIMEOUT;
    loop {
        let now = embassy_time::Instant::now();
        if now >= deadline {
            info!("mDNS: no answer for {}.local", label);
            return None;
        }
        match with_timeout(deadline - now, socket.recv_from(recv)).await {
            Ok(Ok((n, meta))) => {
                let pkt = &recv[..n];
                // Is this the answer we're waiting for?
                if let Some(addr) = parse_answer(pkt, label) {
                    return Some(addr);
                }
                // Otherwise it might be a query for us — stay responsive.
                handle_inbound(socket, group, pkt, meta.endpoint, hostname, stack).await;
            }
            Ok(Err(e)) => warn!("mDNS: recv failed during resolve: {:?}", e),
            Err(_) => {
                info!("mDNS: no answer for {}.local", label);
                return None;
            }
        }
    }
}

/// Responder path: if `pkt` is a query for `hostname.local`, multicast an `A`
/// answer with our current IPv4. Ignored otherwise (answers, other names).
async fn handle_inbound(
    socket: &UdpSocket<'_>,
    group: IpEndpoint,
    pkt: &[u8],
    _from: IpEndpoint,
    hostname: &'static str,
    stack: Stack<'static>,
) {
    if !is_query_for(pkt, hostname) {
        return;
    }
    let Some(ip) = current_ipv4(stack) else {
        return;
    };
    let mut answer = [0u8; 256];
    if let Some(len) = build_answer(&mut answer, hostname, ip) {
        if let Err(e) = socket.send_to(&answer[..len], group).await {
            warn!("mDNS: answer send failed: {:?}", e);
        } else {
            info!("mDNS: answered {}.local -> {:?}", hostname, ip.octets());
        }
    }
}

/// The device's current IPv4, or `None` before DHCP completes.
fn current_ipv4(stack: Stack<'static>) -> Option<Ipv4Address> {
    stack.config_v4().map(|cfg| cfg.address.address())
}

// ---------------------------------------------------------------------------
// DNS wire format (RFC 1035), the minimal subset for single-label `.local` A
// records. Names here are exactly two labels: `<label>` + `local`. Everything is
// uncompressed; we never emit or follow name-compression pointers on the names we
// build, and we tolerate (skip over) them when scanning inbound packets.
// ---------------------------------------------------------------------------

/// DNS `TYPE` for an IPv4 host address (`A`).
const TYPE_A: u16 = 1;
/// DNS `CLASS` `IN` (internet). mDNS sets the top bit for cache-flush / unicast
/// hints; we mask it off when comparing.
const CLASS_IN: u16 = 1;
/// Mask for the cache-flush / unicast-response bit mDNS overloads onto `CLASS`.
const CLASS_MASK: u16 = 0x7fff;

/// Build a multicast `A` query for `label.local` into `buf`, returning its length.
/// `None` if the labels don't fit the DNS length limits.
fn build_query(buf: &mut [u8], label: &str) -> Option<usize> {
    let mut w = Writer::new(buf);
    // Header: id=0, flags=0 (standard query), qd=1, an/ns/ar=0.
    w.u16(0)?; // id
    w.u16(0)?; // flags
    w.u16(1)?; // qdcount
    w.u16(0)?; // ancount
    w.u16(0)?; // nscount
    w.u16(0)?; // arcount
    w.name(label)?;
    w.u16(TYPE_A)?;
    w.u16(CLASS_IN)?;
    Some(w.pos)
}

/// Build an `A` answer advertising `label.local -> ip` into `buf`, returning its
/// length. Sent as an unsolicited authoritative response (QR=1, AA=1), the mDNS
/// convention. `None` if it doesn't fit.
fn build_answer(buf: &mut [u8], label: &str, ip: Ipv4Address) -> Option<usize> {
    let mut w = Writer::new(buf);
    w.u16(0)?; // id (0 for mDNS)
    w.u16(0x8400)?; // flags: QR=1 (response), AA=1 (authoritative)
    w.u16(0)?; // qdcount
    w.u16(1)?; // ancount
    w.u16(0)?; // nscount
    w.u16(0)?; // arcount
    w.name(label)?;
    w.u16(TYPE_A)?;
    // cache-flush bit set + IN, per mDNS for unique records.
    w.u16(0x8000 | CLASS_IN)?;
    w.u32(120)?; // TTL seconds
    w.u16(4)?; // RDLENGTH (an IPv4 is 4 bytes)
    for b in ip.octets() {
        w.u8(b)?;
    }
    Some(w.pos)
}

/// Is `pkt` a query (QR=0) whose question is `hostname.local` type `A`?
fn is_query_for(pkt: &[u8], hostname: &str) -> bool {
    let mut r = match Reader::new(pkt) {
        Some(r) => r,
        None => return false,
    };
    // QR bit must be 0 (a query).
    if r.flags & 0x8000 != 0 {
        return false;
    }
    let qd = r.qdcount;
    if qd == 0 {
        return false;
    }
    // Walk the questions; any matching `hostname.local`/A is a hit.
    for _ in 0..qd {
        let matched = r.name_matches(hostname);
        let (qtype, qclass) = match (r.u16(), r.u16()) {
            (Some(t), Some(c)) => (t, c),
            _ => return false,
        };
        if matched && qtype == TYPE_A && (qclass & CLASS_MASK) == CLASS_IN {
            return true;
        }
    }
    false
}

/// If `pkt` is a response carrying an `A` record for `label.local`, return its
/// IPv4. Scans the answer section.
fn parse_answer(pkt: &[u8], label: &str) -> Option<Ipv4Address> {
    let mut r = Reader::new(pkt)?;
    // QR bit must be 1 (a response).
    if r.flags & 0x8000 == 0 {
        return None;
    }
    // Skip the question section, if any.
    for _ in 0..r.qdcount {
        r.skip_name()?;
        r.u16()?; // qtype
        r.u16()?; // qclass
    }
    // Walk answers looking for our `label.local` A record.
    for _ in 0..r.ancount {
        let matched = r.name_matches(label);
        let rtype = r.u16()?;
        let _rclass = r.u16()?;
        let _ttl = r.u32()?;
        let rdlen = r.u16()? as usize;
        if matched && rtype == TYPE_A && rdlen == 4 {
            let a = r.u8()?;
            let b = r.u8()?;
            let c = r.u8()?;
            let d = r.u8()?;
            return Some(Ipv4Address::new(a, b, c, d));
        }
        r.skip(rdlen)?;
    }
    None
}

/// Tiny forward-only writer over a fixed buffer; every method returns `None` on
/// overflow so a too-long name fails the whole build cleanly.
struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn u8(&mut self, v: u8) -> Option<()> {
        *self.buf.get_mut(self.pos)? = v;
        self.pos += 1;
        Some(())
    }
    fn u16(&mut self, v: u16) -> Option<()> {
        self.u8((v >> 8) as u8)?;
        self.u8(v as u8)
    }
    fn u32(&mut self, v: u32) -> Option<()> {
        self.u16((v >> 16) as u16)?;
        self.u16(v as u16)
    }
    /// Write `<label>.local` as DNS labels terminated by a zero byte. Each label
    /// must be 1..=63 bytes.
    fn name(&mut self, label: &str) -> Option<()> {
        for part in [label, "local"] {
            let bytes = part.as_bytes();
            if bytes.is_empty() || bytes.len() > 63 {
                return None;
            }
            self.u8(bytes.len() as u8)?;
            for &b in bytes {
                self.u8(b)?;
            }
        }
        self.u8(0)
    }
}

/// Forward-only reader over a received packet, holding the parsed header. Bounds-
/// checked: any out-of-range read returns `None`.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
    flags: u16,
    qdcount: u16,
    ancount: u16,
}

impl<'a> Reader<'a> {
    /// Parse the 12-byte header, leaving `pos` at the first question.
    fn new(buf: &'a [u8]) -> Option<Self> {
        if buf.len() < 12 {
            return None;
        }
        let flags = u16::from_be_bytes([buf[2], buf[3]]);
        let qdcount = u16::from_be_bytes([buf[4], buf[5]]);
        let ancount = u16::from_be_bytes([buf[6], buf[7]]);
        Some(Self {
            buf,
            pos: 12,
            flags,
            qdcount,
            ancount,
        })
    }
    fn u8(&mut self) -> Option<u8> {
        let v = *self.buf.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes([
            self.u8()?,
            self.u8()?,
            self.u8()?,
            self.u8()?,
        ]))
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        self.pos = self.pos.checked_add(n)?;
        if self.pos > self.buf.len() {
            return None;
        }
        Some(())
    }
    /// Advance past a DNS name (a run of length-prefixed labels ending in a zero
    /// byte, or terminated by a compression pointer). Does not decode it.
    fn skip_name(&mut self) -> Option<()> {
        loop {
            let len = self.u8()?;
            match len & 0xc0 {
                // Compression pointer (top two bits set): one more byte, then the
                // name ends here.
                0xc0 => {
                    self.u8()?;
                    return Some(());
                }
                0x00 => {
                    if len == 0 {
                        return Some(()); // root label terminates the name
                    }
                    self.skip(len as usize)?;
                }
                // 0x40 / 0x80 length codes are reserved; treat as malformed.
                _ => return None,
            }
        }
    }
    /// Compare the name at `pos` against `<label>.local` case-insensitively,
    /// advancing `pos` past the name either way. Returns whether it matched.
    ///
    /// Only handles uncompressed names (the form responders emit); a name that
    /// uses a compression pointer is skipped and reported as a non-match, which is
    /// safe for our single-question / single-answer matching.
    fn name_matches(&mut self, label: &str) -> bool {
        let expected = [label.as_bytes(), b"local"];
        let mut idx = 0;
        loop {
            let len = match self.u8() {
                Some(l) => l,
                None => return false,
            };
            if len & 0xc0 == 0xc0 {
                // Compression pointer: consume the second byte, can't compare.
                let _ = self.u8();
                return false;
            }
            if len == 0 {
                // End of name: matched iff we consumed exactly the expected labels.
                return idx == expected.len();
            }
            let n = len as usize;
            let start = self.pos;
            if self.skip(n).is_none() {
                return false;
            }
            let got = &self.buf[start..start + n];
            let ok = idx < expected.len()
                && expected[idx].len() == n
                && got
                    .iter()
                    .zip(expected[idx])
                    .all(|(a, b)| a.eq_ignore_ascii_case(b));
            // Keep scanning to leave pos past the name even on mismatch, but record
            // the failure.
            if !ok {
                // Continue consuming labels so pos ends past the name.
                idx = usize::MAX; // poison: can never equal expected.len() at the 0 byte
            } else {
                idx += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_roundtrips_through_is_query_for() {
        let mut buf = [0u8; 256];
        let len = build_query(&mut buf, "beet-esp").unwrap();
        assert!(is_query_for(&buf[..len], "beet-esp"));
        assert!(!is_query_for(&buf[..len], "other"));
    }

    #[test]
    fn answer_roundtrips_through_parse_answer() {
        let ip = Ipv4Address::new(192, 168, 1, 42);
        let mut buf = [0u8; 256];
        let len = build_answer(&mut buf, "beet-esp", ip).unwrap();
        assert_eq!(parse_answer(&buf[..len], "beet-esp"), Some(ip));
        assert_eq!(parse_answer(&buf[..len], "other"), None);
    }

    #[test]
    fn query_is_not_mistaken_for_answer() {
        let mut buf = [0u8; 256];
        let len = build_query(&mut buf, "beet-esp").unwrap();
        // A query (QR=0) must not parse as an answer.
        assert_eq!(parse_answer(&buf[..len], "beet-esp"), None);
    }

    #[test]
    fn answer_is_not_mistaken_for_query() {
        let ip = Ipv4Address::new(10, 0, 0, 5);
        let mut buf = [0u8; 256];
        let len = build_answer(&mut buf, "beet-esp", ip).unwrap();
        // An answer (QR=1) must not be treated as a query to respond to.
        assert!(!is_query_for(&buf[..len], "beet-esp"));
    }

    #[test]
    fn case_insensitive_match() {
        let mut buf = [0u8; 256];
        let len = build_query(&mut buf, "Beet-ESP").unwrap();
        assert!(is_query_for(&buf[..len], "beet-esp"));
    }
}
