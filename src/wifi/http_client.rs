//! beet's `Request::get(url).send().await` client transport, over Wi-Fi.
//!
//! The ESP32 transport is registered with beet via [`set_http_client`] (by
//! [`start_wifi`](super::start_wifi)), so calling [`Request::send`] anywhere
//! routes through [`esp_send`]: the request is encoded to HTTP/1.1, handed over
//! a [`static`](CLIENT_BRIDGE) [`AsyncBridge`] to [`client_driver`], and the
//! awaited [`Response`] comes back on that call's reply.
//!
//! Pure transport — no `action` dependency. The encode/parse here is the
//! *client* direction (encode a [`Request`], parse a [`Response`]); the
//! *server* direction reuses beet's shared `http_ext` helpers.

use crate::async_bridge::AsyncBridge;
use beet::prelude::*;
use defmt::info;
use defmt::warn;
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

        let (parts, body) = request.into_parts();
        let body = body.into_bytes().await?;
        let bytes = encode_request(&parts, &authority, &body);

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
pub(crate) async fn client_driver(stack: Stack<'static>) {
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("Wi-Fi up: {}", cfg);
    }
    loop {
        let ex = CLIENT_BRIDGE.recv().await;
        let (job, reply) = ex.split();
        reply.send(exchange(stack, &job).await);
    }
}

/// Resolve the host, open a socket, send the encoded request, and read the whole
/// response into a beet [`Response`].
async fn exchange(stack: Stack<'static>, job: &ClientJob) -> Result<Response> {
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
    Ok(parse_response(&raw))
}

// ---------------------------------------------------------------------------
// HTTP/1.1 wire helpers (client direction: encode Request, parse Response)
// ---------------------------------------------------------------------------

/// Split an authority into `(host, port)`, falling back to `default_port`.
fn split_authority(authority: &str, default_port: u16) -> (String, u16) {
    match authority.rsplit_once(':') {
        Some((host, port)) => (host.to_string(), port.parse().unwrap_or(default_port)),
        None => (authority.to_string(), default_port),
    }
}

/// The uppercase HTTP token for a method (the [`HttpMethod`] `Display` is
/// title-case, e.g. `Get`, so it can't be used on the wire).
fn method_token(method: &HttpMethod) -> &'static str {
    match method {
        HttpMethod::Get => "GET",
        HttpMethod::Post => "POST",
        HttpMethod::Put => "PUT",
        HttpMethod::Patch => "PATCH",
        HttpMethod::Delete => "DELETE",
        HttpMethod::Options => "OPTIONS",
        HttpMethod::Head => "HEAD",
        HttpMethod::Trace => "TRACE",
        HttpMethod::Connect => "CONNECT",
    }
}

/// Headers the encoder sets itself; user-supplied copies are skipped to avoid
/// duplicates.
fn is_managed_header(key: &str) -> bool {
    key.eq_ignore_ascii_case("host")
        || key.eq_ignore_ascii_case("content-length")
        || key.eq_ignore_ascii_case("connection")
}

/// Serialise a [`Request`] to raw HTTP/1.1 bytes (origin-form target).
fn encode_request(parts: &RequestParts, authority: &str, body: &[u8]) -> Vec<u8> {
    let path = parts.path_string();
    let query = parts.query_string();
    let target = if query.is_empty() {
        path
    } else {
        format!("{path}?{query}")
    };

    let mut head = format!("{} {} HTTP/1.1\r\n", method_token(parts.method()), target);
    head.push_str(&format!("Host: {authority}\r\n"));
    for (key, values) in parts.headers().iter_all() {
        if is_managed_header(key) {
            continue;
        }
        for value in values {
            head.push_str(&format!("{key}: {value}\r\n"));
        }
    }
    head.push_str(&format!("Content-Length: {}\r\n", body.len()));
    head.push_str("Connection: close\r\n\r\n");

    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// Parse raw HTTP/1.1 response bytes into a beet [`Response`].
fn parse_response(raw: &[u8]) -> Response {
    let (header_section, body) = split_head_body(raw);
    let header_str = String::from_utf8_lossy(header_section);
    let mut lines = header_str.lines();

    let status = lines.next().and_then(parse_status_line).unwrap_or(0);
    let mut parts = ResponseParts::new(StatusCode::new(status));
    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            parts.headers.set_raw(key.trim(), value.trim());
        }
    }
    Response::new(parts, body.to_vec().into())
}

/// Pull the status code out of an HTTP status line (`HTTP/1.1 200 OK`).
fn parse_status_line(line: &str) -> Option<u16> {
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Split a raw HTTP message into `(headers, body)` on the blank-line separator.
fn split_head_body(raw: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = find_subslice(raw, b"\r\n\r\n") {
        (&raw[..pos], &raw[pos + 4..])
    } else if let Some(pos) = find_subslice(raw, b"\n\n") {
        (&raw[..pos], &raw[pos + 2..])
    } else {
        (raw, &[])
    }
}

/// First index of `needle` within `haystack`.
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
