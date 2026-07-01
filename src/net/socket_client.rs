//! beet's `Socket::connect(url).await` WebSocket client transport, over Wi-Fi.
//!
//! The socket analogue of [`http_client`](super::http_client): registered with
//! beet via [`set_socket_client`] (by [`start_wifi`](super::start_wifi)), so
//! [`Socket::connect`] anywhere routes through [`esp_connect`].
//!
//! Unlike the one-shot HTTP client, a socket is long-lived and bidirectional, so
//! this is not a `run_worker` request/reply. [`esp_connect`] creates a pair of
//! `embassy_sync` channels, queues the `(host, port, channel ends)` for
//! [`socket_driver`], and immediately builds and returns a [`Socket`] whose
//! reader/writer are just those channel ends (the `impl_web_sys` shape).
//! [`socket_driver`] owns the [`Stack`], and per connection opens a `TcpSocket`
//! (with locally-owned buffers, so no `'static` puzzle), performs the RFC 6455
//! handshake, then runs a `select` loop: decode inbound frames to the reader,
//! drain the outbound channel to the wire.

use crate::esp32_utils::async_bridge::Queue;
use crate::net::http_client::split_authority;
use alloc::sync::Arc;
use beet::prelude::*;
use beet::prelude::sockets::*;
// disambiguate the socket `Message` enum from bevy's `Message` trait (both are
// glob-imported above), matching the upstream socket examples.
use beet::prelude::sockets::Message;
use core::pin::Pin;
use core::task::Context;
use core::task::Poll;
use embassy_futures::select::Either;
use embassy_futures::select::select;
use embassy_net::IpEndpoint;
use embassy_net::Stack;
use embassy_net::dns::DnsQueryType;
use embassy_net::tcp::TcpSocket;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_time::Duration;
use embassy_time::with_timeout;
use embedded_io_async::Write as _;
use esp_hal::rng::Rng;
use futures_core::Stream;

/// Depth of the per-connection inbound/outbound channels. A few in-flight
/// messages is ample for request/response socket traffic.
const CHANNEL_DEPTH: usize = 8;

/// Inbound (socket → world) decoded messages, feeding the [`Socket`] reader.
type InboundChannel = Channel<CriticalSectionRawMutex, Result<Message>, CHANNEL_DEPTH>;
/// Outbound (world → socket) messages, drained by the driver onto the wire.
type OutboundChannel = Channel<CriticalSectionRawMutex, Message, CHANNEL_DEPTH>;

/// A pending connection handed from [`esp_connect`] to [`socket_driver`]: the
/// target and the channel ends shared with the returned [`Socket`].
struct ConnectJob {
	/// The `host:port` authority for the handshake `Host` header.
	authority: String,
	/// Host to resolve (an IP literal or a DNS name).
	host: String,
	/// TCP port to connect to.
	port: u16,
	inbound: Arc<InboundChannel>,
	outbound: Arc<OutboundChannel>,
}

/// Connect jobs queued for [`socket_driver`]. Depth 2 is plenty: the device
/// opens sockets one at a time.
static SOCKET_QUEUE: Queue<ConnectJob, 2> = Queue::new();

/// beet's socket transport hook (see [`set_socket_client`]): parse the target,
/// create the connection channels, queue the job for [`socket_driver`], and
/// return the [`Socket`] immediately. The driver connects and pumps frames.
///
/// Installed once by [`start_wifi`](super::start_wifi); thereafter any
/// `Socket::connect` flows through here.
pub(crate) fn esp_connect(
	url: &str,
) -> MaybeSendBoxedFuture<'static, Result<Socket>> {
	let url = url.to_string();
	Box::pin(async move {
		let authority = resolve_authority(&url)?;
		let (host, port) = split_authority(&authority, 80);
		let inbound = Arc::new(InboundChannel::new());
		let outbound = Arc::new(OutboundChannel::new());
		let job = ConnectJob {
			authority,
			host,
			port,
			inbound: inbound.clone(),
			outbound: outbound.clone(),
		};
		SOCKET_QUEUE
			.send(job)
			.map_err(|_| bevyhow!("Wi-Fi socket connect queue full"))?;
		Ok(Socket::new(
			InboundStream { inbound },
			OutboundWriter { outbound },
		))
	})
}

/// Services [`SOCKET_QUEUE`]: one connection at a time. Owns a copy of the
/// [`Stack`]; waits for DHCP before the first job. Each job connects, handshakes
/// and runs the duplex loop until the socket closes, then loops for the next.
pub(crate) async fn socket_driver(stack: Stack<'static>) {
	stack.wait_config_up().await;
	loop {
		let job = SOCKET_QUEUE.recv().await;
		info!("socket connecting to `{}`", job.authority.as_str());
		if let Err(err) = handle_connection(stack, &job).await {
			warn!("socket connection failed: {:?}", err);
			// surface the failure to the reader so the app's stream sees it.
			job.inbound.send(Err(err)).await;
		}
	}
}

/// Resolve the connect target's authority (`host:port`) from an explicit url or
/// the `SOCKET_SERVER` env default, accepting a bare `host:port` or a `ws://` url.
fn resolve_authority(url: &str) -> Result<String> {
	let target = if url.is_empty() {
		option_env!("BEET_SOCKET_SERVER").ok_or_else(|| {
			bevyhow!(
				"Socket::connect got an empty url and BEET_SOCKET_SERVER is unset"
			)
		})?
	} else {
		url
	};
	if target.starts_with("wss://") {
		bevybail!("wss (TLS) is not supported by the esp socket transport");
	}
	Ok(target.strip_prefix("ws://").unwrap_or(target).to_string())
}

/// Open the socket, perform the RFC 6455 handshake, and run the duplex frame
/// loop for one connection. All buffers live in this task's frame, so the
/// borrowed `TcpSocket` never needs a `'static` buffer.
async fn handle_connection(stack: Stack<'static>, job: &ConnectJob) -> Result<()> {
	let remote = resolve_endpoint(stack, &job.host, job.port).await?;
	info!("resolved `{}` -> {:?}", job.authority.as_str(), remote);
	let mut rx = [0u8; 1536];
	let mut tx = [0u8; 1536];
	let mut socket = TcpSocket::new(stack, &mut rx, &mut tx);
	socket.set_timeout(Some(Duration::from_secs(30)));
	// Bound the connect so an unreachable or firewalled server errors cleanly
	// instead of hanging the socket forever.
	with_timeout(Duration::from_secs(10), socket.connect(remote))
		.await
		.map_err(|_| {
			bevyhow!(
				"connect to {} timed out (server unreachable or firewalled?)",
				job.authority.as_str()
			)
		})?
		.map_err(|err| {
			bevyhow!("connect to {} failed: {:?}", job.authority.as_str(), err)
		})?;
	info!("tcp connected to `{}`, sending handshake", job.authority.as_str());

	// RFC 6455 client handshake: send the upgrade request, validate the 101.
	let key = ws_ext::encode_client_key(random_bytes::<16>());
	let request = ws_ext::encode_handshake_request(&job.authority, "/", &key)?;
	socket
		.write_all(&request)
		.await
		.map_err(|err| bevyhow!("handshake write failed: {:?}", err))?;
	let leftover = read_handshake_response(&mut socket, &key).await?;

	info!("socket connected to `{}`", job.authority.as_str());
	run_duplex(&mut socket, job, leftover).await
}

/// Read the server's handshake response up to the header terminator, validate
/// the `101`, and return any bytes read past the headers (the first frame data).
async fn read_handshake_response(
	socket: &mut TcpSocket<'_>,
	key: &str,
) -> Result<Vec<u8>> {
	let mut response = Vec::new();
	let mut buf = [0u8; 512];
	loop {
		let n = socket
			.read(&mut buf)
			.await
			.map_err(|err| bevyhow!("handshake read failed: {:?}", err))?;
		if n == 0 {
			bevybail!("server closed the connection during the handshake");
		}
		response.extend_from_slice(&buf[..n]);
		if let Some(end) = http_ext::find_header_end(&response) {
			ws_ext::validate_handshake_response(&response[..end], key)?;
			return Ok(response[end..].to_vec());
		}
	}
}

/// The bidirectional frame pump: decode inbound frames to the reader, drain the
/// outbound channel to the wire, until either side closes.
async fn run_duplex(
	socket: &mut TcpSocket<'_>,
	job: &ConnectJob,
	mut decode_buf: Vec<u8>,
) -> Result<()> {
	let mut read_buf = [0u8; 512];
	loop {
		// drain any complete frames already buffered before awaiting more IO.
		while let Some((message, consumed)) = ws_ext::parse_frame(&decode_buf)? {
			decode_buf.drain(..consumed);
			let is_close = matches!(message, Message::Close(_));
			job.inbound.send(Ok(message)).await;
			if is_close {
				return Ok(());
			}
		}
		match select(socket.read(&mut read_buf), job.outbound.receive()).await {
			Either::First(read) => {
				let n =
					read.map_err(|err| bevyhow!("socket read failed: {:?}", err))?;
				if n == 0 {
					// remote closed the TCP stream without a Close frame.
					job.inbound.send(Ok(Message::Close(None))).await;
					return Ok(());
				}
				decode_buf.extend_from_slice(&read_buf[..n]);
			}
			Either::Second(message) => {
				let is_close = matches!(message, Message::Close(_));
				// client frames are masked with a fresh random key (RFC 6455 §5.3).
				let frame =
					ws_ext::encode_frame(&message, Some(random_bytes::<4>()));
				socket
					.write_all(&frame)
					.await
					.map_err(|err| bevyhow!("socket write failed: {:?}", err))?;
				if is_close {
					return Ok(());
				}
			}
		}
	}
}

/// Resolve `host` to an [`IpEndpoint`]. `dns_query` short-circuits IP literals,
/// covering the `SOCKET_SERVER` IP case and real hostnames alike.
async fn resolve_endpoint(
	stack: Stack<'static>,
	host: &str,
	port: u16,
) -> Result<IpEndpoint> {
	let addrs = stack
		.dns_query(host, DnsQueryType::A)
		.await
		.map_err(|err| bevyhow!("DNS lookup for `{host}` failed: {:?}", err))?;
	let addr = addrs
		.into_iter()
		.next()
		.ok_or_else(|| bevyhow!("no DNS results for `{host}`"))?;
	Ok(IpEndpoint::new(addr, port))
}

/// A fresh array of hardware-RNG bytes for the client mask key / handshake key.
fn random_bytes<const N: usize>() -> [u8; N] {
	let rng = Rng::new();
	let mut out = [0u8; N];
	for chunk in out.chunks_mut(4) {
		let word = rng.random().to_le_bytes();
		chunk.copy_from_slice(&word[..chunk.len()]);
	}
	out
}

/// The [`Socket`] reader: a stream over the inbound channel the driver pushes
/// decoded [`Message`]s into. It never ends (the driver pushes a terminal
/// `Close`/error item), matching the browser channel-fed reader.
struct InboundStream {
	inbound: Arc<InboundChannel>,
}

impl Stream for InboundStream {
	type Item = Result<Message>;
	fn poll_next(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Self::Item>> {
		self.get_mut().inbound.poll_receive(cx).map(Some)
	}
}

/// The [`Socket`] writer: enqueues [`Message`]s onto the outbound channel the
/// driver drains and frames onto the wire. Models `impl_web_sys`'s channel-style
/// writer; sends resolve once the message is queued.
struct OutboundWriter {
	outbound: Arc<OutboundChannel>,
}

impl SocketWriter for OutboundWriter {
	fn send_boxed(&mut self, msg: Message) -> SendBoxedFuture<Result<()>> {
		let outbound = self.outbound.clone();
		Box::pin(async move {
			outbound.send(msg).await;
			Ok(())
		})
	}
	fn close_boxed(
		&mut self,
		close: Option<CloseFrame>,
	) -> SendBoxedFuture<Result<()>> {
		let outbound = self.outbound.clone();
		Box::pin(async move {
			outbound.send(Message::Close(close)).await;
			Ok(())
		})
	}
}
