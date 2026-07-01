//! beet's `Request::get(url).send().await` client transport, over Wi-Fi.
//!
//! The ESP32 transport is registered with beet via [`set_http_client`] (by
//! [`start_wifi`](super::start_wifi)), so calling [`Request::send`] anywhere
//! routes through [`esp_send`]: the request is encoded to HTTP/1.1, handed over
//! a [`static`](CLIENT_BRIDGE) [`AsyncBridge`] to [`client_driver`], and the
//! awaited [`Response`] comes back on that call's reply.
//!
//! Pure transport — no `action` dependency. The HTTP/1.1 wire encode/parse lives
//! upstream in beet's shared [`http_ext`] module (`encode_request` /
//! `parse_response`); this file is just the bridge plus the socket IO.

use crate::esp32_utils::async_bridge::AsyncBridge;
use crate::esp32_utils::async_bridge::run_worker;
use beet::prelude::*;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Stack;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_time::Duration;
use embedded_io_async::Write as _;

/// Pending client requests handed from [`esp_send`] to [`client_driver`], each
/// awaiting the [`Response`] the driver produces (or the [`BevyError`] it hit).
static CLIENT_BRIDGE: AsyncBridge<ClientJob, Result<Response, BevyError>, 4> =
    AsyncBridge::new();

/// A request encoded and queued for the driver.
struct ClientJob {
    /// Authority host to resolve (an IP literal or a DNS name).
    host: String,
    /// TCP port to connect to.
    port: u16,
    /// The full HTTP/1.1 request, ready to write to the socket.
    bytes: Vec<u8>,
}

/// beet's transport hook (see [`set_http_client`]): encode the [`Request`], queue
/// it for [`client_driver`], and await the [`Response`].
///
/// Installed once by [`start_wifi`](super::start_wifi); thereafter any
/// `Request::send` on an `http://` URL flows through here.
pub(crate) fn esp_send(
    request: Request,
) -> MaybeSendBoxedFuture<'static, Result<Response>> {
    Box::pin(async move {
        if request.scheme().is_secure() {
            bevybail!("the esp32 Wi-Fi transport only supports plain http, not https/tls");
        }
        let authority = request.authority().to_string();
        if authority.is_empty() {
            bevybail!("request URL has no host: {}", request.uri());
        }
        let (host, port) = split_authority(&authority, 80);

        // Collect any streamed body into memory so the wire encoder (which only
        // serialises `Body::Bytes`) can handle it, then encode to HTTP/1.1.
        let (parts, body) = request.into_parts();
        let body = body.into_bytes().await?;
        let request = Request::from_parts(parts, body.into());
        let bytes = http_ext::encode_request(&request, Default::default())?;

        let job = ClientJob { host, port, bytes };
        // Outer `Err` is a full queue (no reply will arrive); the inner
        // `Result<Response, BevyError>` is the actual transport outcome.
        CLIENT_BRIDGE
            .call(job)
            .await
            .map_err(|_| bevyhow!("Wi-Fi client queue full"))?
    })
}

/// Services [`CLIENT_BRIDGE`]: one TCP request at a time, sending the result back
/// on each call's reply. Owns a copy of the [`Stack`]; waits for DHCP before the
/// first job.
///
/// The recv/reply loop is the request/reply toolkit's [`run_worker`]; all this
/// adds is the one-time DHCP wait and the per-job `worker` ([`exchange`]).
pub(crate) async fn client_driver(stack: Stack<'static>) {
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("Wi-Fi up: {:?}", cfg);
    }
    run_worker(&CLIENT_BRIDGE, |job: ClientJob| exchange(stack, job)).await;
}

/// Resolve the host, open a socket, send the encoded request, and read the whole
/// response into a beet [`Response`].
async fn exchange(stack: Stack<'static>, job: ClientJob) -> Result<Response> {
    // `.local` names are not in unicast DNS; resolve them over mDNS multicast via
    // the responder/resolver task (only present under the `mdns` feature, and only
    // running if an `MDns` server is up). Falls through to unicast DNS otherwise,
    // so a `.local` lookup with no mDNS task simply fails the normal way.
    #[cfg(feature = "mdns")]
    if job.host.ends_with(".local") {
        info!("resolving `{}` via mDNS", job.host.as_str());
        let addr = super::mdns::resolve(&job.host).await.ok_or_else(|| {
            bevyhow!("mDNS lookup for `{}` failed (no answer)", job.host.as_str())
        })?;
        info!("mDNS resolved `{}` -> {:?}", job.host.as_str(), addr.octets());
        let remote = IpEndpoint::new(IpAddress::Ipv4(addr), job.port);
        return exchange_on(stack, remote, job).await;
    }

    // `dns_query` short-circuits IP literals, so this covers both `1.1.1.1` and
    // real hostnames (DNS servers come from the DHCP lease).
    let addrs = stack
        .dns_query(&job.host, DnsQueryType::A)
        .await
        .map_err(|e| bevyhow!("DNS lookup for `{}` failed: {:?}", job.host.as_str(), e))?;
    let addr: IpAddress = addrs
        .into_iter()
        .next()
        .ok_or_else(|| bevyhow!("no DNS results for `{}`", job.host.as_str()))?;
    let remote = IpEndpoint::new(addr, job.port);
    exchange_on(stack, remote, job).await
}

/// Open a TCP socket to `remote`, send the encoded request, and read the whole
/// response into a beet [`Response`]. The transport tail shared by the unicast-DNS
/// and `.local`/mDNS resolution paths.
async fn exchange_on(
    stack: Stack<'static>,
    remote: IpEndpoint,
    job: ClientJob,
) -> Result<Response> {
    let mut rx = [0u8; 1536];
    let mut tx = [0u8; 1536];
    let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
    socket.set_timeout(Some(Duration::from_secs(10)));

    socket
        .connect(remote)
        .await
        .map_err(|e| bevyhow!("connect to {} failed: {:?}", job.host.as_str(), e))?;
    socket
        .write_all(&job.bytes)
        .await
        .map_err(|e| bevyhow!("write failed: {:?}", e))?;

    let mut raw = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        match socket.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(e) => {
                warn!("read failed: {:?}", e);
                break;
            }
        }
    }
    http_ext::parse_response(&raw)
}

/// Split an authority into `(host, port)`, falling back to `default_port`.
pub(crate) fn split_authority(authority: &str, default_port: u16) -> (String, u16) {
    match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(default_port)),
        None => (authority.to_string(), default_port),
    }
}
