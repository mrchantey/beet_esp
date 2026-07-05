//! beet's `Socket::connect(url).await` WebSocket client transport, over Wi-Fi.
//!
//! The socket analogue of [`http_client`](super::http_client): registered with
//! beet via [`set_socket_client`] (by [`start_wifi`](super::start_wifi)), so
//! [`Socket::connect`] anywhere routes through [`esp_connect`].
//!
//! One call is one connection, matching the tungstenite transport's semantics:
//! [`esp_connect`] queues a [`ConnectJob`] for [`socket_driver`] and awaits the
//! dial + RFC 6455 handshake outcome, so a failed connect is the caller's
//! `Err`. The returned [`Socket`]'s reader ends when the connection does (the
//! driver drops its inbound sender), which upstream becomes [`SocketClosed`] —
//! redial policy and disconnect reactions are upstream concerns (eg
//! [`PersistentSocket`] + `ResetOnDisconnect`), not this transport's.
//!
//! A `wss://` url (the `secure` feature) wraps the `TcpSocket` in the
//! pinned-cert TLS session from [`tls_client`](super::tls_client); the
//! handshake + frame pump are generic over the transport.

use crate::esp32_utils::async_bridge::AsyncBridge;
use crate::esp32_utils::async_bridge::Reply;
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
use embedded_io_async::Read;
use embedded_io_async::Write;
use esp_hal::rng::Rng;
use futures_core::Stream;

/// Depth of the per-connection outbound channel. A few in-flight messages is
/// ample for request/response socket traffic.
const CHANNEL_DEPTH: usize = 8;

/// Outbound (world → socket) messages, drained by the driver onto the wire.
type OutboundChannel = Channel<CriticalSectionRawMutex, Message, CHANNEL_DEPTH>;
/// Inbound (socket → world) decoded messages, feeding the [`Socket`] reader.
/// beet's agnostic mpsc, whose receiver ends when the sender drops — so "the
/// driver task finished" *is* "the socket stream ended".
type InboundSender = writer_channel::Sender<Result<Message>>;

/// A connection handed from [`esp_connect`] to [`socket_driver`]: the target
/// plus the channel ends shared with the returned [`Socket`].
struct ConnectJob {
	/// The `host:port` authority for the handshake `Host` header.
	authority: String,
	/// Host to resolve (an IP literal or a DNS name).
	host: String,
	/// TCP port to connect to.
	port: u16,
	/// Whether to wrap the transport in the pinned-cert TLS session (`wss://`).
	secure: bool,
	inbound: InboundSender,
	outbound: Arc<OutboundChannel>,
}

/// Connect calls awaiting their dial + handshake outcome. Depth 2 is plenty:
/// the device opens sockets one at a time.
static CONNECT_BRIDGE: AsyncBridge<ConnectJob, Result<()>, 2> =
	AsyncBridge::new();

/// beet's socket transport hook (see [`set_socket_client`]): parse the target,
/// create the connection channels, queue the job for [`socket_driver`], and
/// await the dial + handshake before returning the [`Socket`] — a failed
/// connect is this call's `Err`, and the redial policy lives with the caller
/// (eg [`PersistentSocket`]).
///
/// Installed once by [`start_wifi`](super::start_wifi); thereafter any
/// `Socket::connect` flows through here.
pub(crate) fn esp_connect(
	url: Url,
) -> MaybeSendBoxedFuture<'static, Result<Socket>> {
	Box::pin(async move {
		let url = resolve_url(url)?;
		let secure = url.scheme().is_secure();
		let authority = url
			.authority()
			.ok_or_else(|| bevyhow!("socket url has no authority: {url}"))?
			.to_string();
		// a bracketed ipv6 host is unwrapped for DNS
		let host = url
			.host()
			.unwrap_or_default()
			.trim_matches(['[', ']'])
			.to_string();
		let port = url.port_or_default().unwrap_or(match secure {
			true => 443,
			false => 80,
		});
		let (inbound_send, inbound_recv) =
			writer_channel::unbounded::<Result<Message>>();
		let outbound = Arc::new(OutboundChannel::new());
		let job = ConnectJob {
			authority,
			host,
			port,
			secure,
			inbound: inbound_send,
			outbound: outbound.clone(),
		};
		CONNECT_BRIDGE
			.call(job)
			.await
			.map_err(|_| bevyhow!("Wi-Fi socket connect queue full"))??;
		Ok(Socket::new(
			InboundStream {
				inbound: inbound_recv,
			},
			OutboundWriter { outbound },
		))
	})
}

/// Services [`CONNECT_BRIDGE`]: waits for DHCP, then hands each job its own
/// [`drive_connection`] task, so one connection's pump never starves another's
/// dial.
pub(crate) async fn socket_driver(stack: Stack<'static>) {
	stack.wait_config_up().await;
	// SAFETY: `socket_driver` is itself spawned on the embassy executor (see
	// `start_wifi`'s `spawn_driver`), so the current-executor lookup is made
	// from within a task as the contract requires.
	let spawner =
		unsafe { embassy_executor::Spawner::for_current_executor() }.await;
	loop {
		let (job, reply) = CONNECT_BRIDGE.recv().await.split();
		crate::esp32_utils::async_bridge::spawn_driver(
			spawner,
			drive_connection(stack, job, reply),
		);
	}
}

/// The connect target: the given url, or the `BEET_SOCKET_SERVER` build env
/// default when it carries no authority (a bare `host:port` there is prefixed
/// as `ws://`). A `wss://` target requires the `secure` feature.
fn resolve_url(url: Url) -> Result<Url> {
	let url = match url.authority() {
		Some(_) => url,
		None => option_env!("BEET_SOCKET_SERVER")
			.ok_or_else(|| {
				bevyhow!(
					"Socket::connect got no authority and BEET_SOCKET_SERVER is unset"
				)
			})?
			.xmap(|target| match target.contains("://") {
				true => Url::parse(target),
				false => Url::parse(format!("ws://{target}")),
			}),
	};
	if url.scheme().is_secure() && !cfg!(feature = "secure") {
		bevybail!(
			"wss requires this firmware to be built with the `secure` feature"
		);
	}
	Ok(url)
}

/// Drive one connection to completion: dial + handshake (answering `reply`
/// with the outcome), then pump the duplex until the connection ends. Dropping
/// the job's inbound sender on return ends the [`Socket`] stream, which the
/// upstream reader task turns into [`SocketClosed`].
async fn drive_connection(
	stack: Stack<'static>,
	job: ConnectJob,
	reply: Reply<Result<()>>,
) {
	let mut reply = Some(reply);
	let result = handle_connection(stack, &job, &mut reply).await;
	match (reply.take(), result) {
		// dial or handshake failed: the error belongs to the connect caller
		(Some(reply), Err(err)) => reply.send(Err(err)),
		// defensive: every success path replies at handshake time
		(Some(reply), Ok(())) => reply.send(Ok(())),
		// transport died mid-connection: surface it on the socket's stream
		(None, Err(err)) => job.inbound.send(Err(err)),
		(None, Ok(())) => {
			info!("socket to `{}` closed", job.authority.as_str())
		}
	}
}

/// Open the transport (TCP, TLS-wrapped for a secure job) and run the
/// handshake + frame pump for one connection. All buffers live in this task's
/// frame, so the borrowed `TcpSocket` never needs a `'static` buffer.
async fn handle_connection(
	stack: Stack<'static>,
	job: &ConnectJob,
	reply: &mut Option<Reply<Result<()>>>,
) -> Result<()> {
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
	info!("tcp connected to `{}`", job.authority.as_str());

	#[cfg(feature = "secure")]
	if job.secure {
		let mut read_buf = super::tls_client::read_buf();
		let mut write_buf = super::tls_client::write_buf();
		let mut tls =
			super::tls_client::connect(socket, &mut read_buf, &mut write_buf)
				.await?;
		info!("tls session opened with `{}`", job.authority.as_str());
		return run_socket(&mut tls, job, reply).await;
	}
	run_socket(&mut socket, job, reply).await
}

/// The RFC 6455 client handshake then the frame pump, generic over the
/// transport (plain `TcpSocket` or a TLS session).
async fn run_socket<S: Read + Write>(
	socket: &mut S,
	job: &ConnectJob,
	reply: &mut Option<Reply<Result<()>>>,
) -> Result<()> {
	// RFC 6455 client handshake: send the upgrade request, validate the 101.
	let key = ws_ext::encode_client_key(random_bytes::<16>());
	let request = ws_ext::encode_handshake_request(&job.authority, "/", &key)?;
	socket
		.write_all(&request)
		.await
		.map_err(|err| bevyhow!("handshake write failed: {:?}", err))?;
	socket
		.flush()
		.await
		.map_err(|err| bevyhow!("handshake flush failed: {:?}", err))?;
	let leftover = read_handshake_response(socket, &key).await?;

	info!("socket connected to `{}`", job.authority.as_str());
	// the connect caller gets its `Socket` the moment the handshake lands
	if let Some(reply) = reply.take() {
		reply.send(Ok(()));
	}
	run_duplex(socket, job, leftover).await
}

/// Read the server's handshake response up to the header terminator, validate
/// the `101`, and return any bytes read past the headers (the first frame data).
async fn read_handshake_response<S: Read>(
	socket: &mut S,
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
async fn run_duplex<S: Read + Write>(
	socket: &mut S,
	job: &ConnectJob,
	mut decode_buf: Vec<u8>,
) -> Result<()> {
	let mut read_buf = [0u8; 512];
	loop {
		// drain any complete frames already buffered before awaiting more IO.
		while let Some((message, consumed)) = ws_ext::parse_frame(&decode_buf)? {
			decode_buf.drain(..consumed);
			let is_close = matches!(message, Message::Close(_));
			job.inbound.send(Ok(message));
			if is_close {
				return Ok(());
			}
		}
		match select(socket.read(&mut read_buf), job.outbound.receive()).await {
			Either::First(read) => {
				let n =
					read.map_err(|err| bevyhow!("socket read failed: {:?}", err))?;
				if n == 0 {
					// remote closed the stream without a Close frame; the ended
					// inbound channel tells the reader task the same thing.
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
				socket
					.flush()
					.await
					.map_err(|err| bevyhow!("socket flush failed: {:?}", err))?;
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

/// The [`Socket`] reader: yields the driver's decoded messages and ends when
/// the driver drops its sender (the connection is over), so the upstream
/// reader task fires [`SocketClosed`] — the seam `PersistentSocket` and
/// `ResetOnDisconnect` build on.
struct InboundStream {
	inbound: writer_channel::Receiver<Result<Message>>,
}

impl Stream for InboundStream {
	type Item = Result<Message>;
	fn poll_next(
		self: Pin<&mut Self>,
		cx: &mut Context<'_>,
	) -> Poll<Option<Self::Item>> {
		self.get_mut().inbound.poll_recv(cx)
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
