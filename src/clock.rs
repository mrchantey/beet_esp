//! Wall-clock time over SNTP, wired into beet's [`time_ext`] clock hook.
//!
//! A bare ESP32 has a monotonic clock (embassy `Instant`) but no wall clock —
//! nothing knows the date until something tells it. beet exposes a hook for
//! exactly this: [`time_ext::set_now`] installs the function backing
//! [`time_ext::now`] / [`time_ext::try_now`]. This module is the ESP32 adapter
//! that fills it in.
//!
//! [`ClockPlugin`] spawns a task that, once Wi-Fi is up, asks an NTP server for
//! the time (a single 48-byte UDP exchange — RFC 4330 SNTP), anchors it against
//! the monotonic clock, and installs the `now` source. Until that first sync
//! lands, [`time_ext::try_now`] returns an error (and [`time_ext::now`] panics),
//! so callers can tell "still loading" from a real time. The task re-syncs
//! hourly to bound drift.
//!
//! Requires [`WifiPlugin`](crate::wifi::WifiPlugin) (it shares the network
//! [`Stack`]); add `ClockPlugin` after it.

use crate::bridge::spawn_driver;
use beet::prelude::*;
use core::cell::Cell;
use core::sync::atomic::AtomicBool;
use core::sync::atomic::Ordering;
use core::time::Duration;
use critical_section::Mutex;
use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Stack;
use embassy_net::dns::DnsQueryType;
use embassy_net::udp::PacketMetadata;
use embassy_net::udp::UdpSocket;
use embassy_time::Duration as EmbassyDuration;
use embassy_time::Instant;
use embassy_time::Timer;
use embassy_time::with_timeout;

/// Default NTP server, resolved via the DHCP-provided DNS.
const DEFAULT_NTP_SERVER: &str = "pool.ntp.org";
/// Seconds between the NTP epoch (1900-01-01) and the Unix epoch (1970-01-01).
const NTP_UNIX_OFFSET_SECS: u64 = 2_208_988_800;
/// How long to wait for an NTP reply before giving up on a sync attempt.
const SNTP_TIMEOUT: EmbassyDuration = EmbassyDuration::from_secs(5);
/// How long to wait before retrying after a failed sync.
const RETRY_DELAY: EmbassyDuration = EmbassyDuration::from_secs(10);
/// Interval between successful syncs — re-poll to bound oscillator drift.
const RESYNC_INTERVAL: EmbassyDuration = EmbassyDuration::from_secs(3600);

/// Unix time, in milliseconds, that corresponds to monotonic `Instant` zero
/// (device boot). `now = BOOT_UNIX_MILLIS + Instant::now().as_millis()`. Updated
/// on every sync, so re-syncs re-discipline the clock without reinstalling.
///
/// A `critical_section` cell rather than an atomic: Xtensa has no native 64-bit
/// atomics, and the access is rare (once per sync / per `now` call).
static BOOT_UNIX_MILLIS: Mutex<Cell<i64>> = Mutex::new(Cell::new(0));
/// Set once the first sync has populated [`BOOT_UNIX_MILLIS`] and the `now`
/// source has been installed.
static SYNCED: AtomicBool = AtomicBool::new(false);

/// Update the boot anchor (the Unix-millis value at monotonic time zero).
fn store_anchor(boot_unix_millis: i64) {
    critical_section::with(|cs| BOOT_UNIX_MILLIS.borrow(cs).set(boot_unix_millis));
}

/// The [`NowFn`] installed via [`time_ext::set_now`]: the boot anchor plus
/// monotonic uptime.
fn sntp_now() -> Duration {
    let boot = critical_section::with(|cs| BOOT_UNIX_MILLIS.borrow(cs).get());
    let uptime = Instant::now().as_millis() as i64;
    Duration::from_millis((boot + uptime).max(0) as u64)
}

/// Plugin that keeps wall-clock time synced over SNTP and serves it through
/// beet's [`time_ext`] clock hook. Add after
/// [`WifiPlugin`](crate::wifi::WifiPlugin).
pub struct ClockPlugin {
    server: &'static str,
}

impl ClockPlugin {
    /// Sync against the given NTP server (host name or IP literal).
    pub fn new(server: &'static str) -> Self {
        Self { server }
    }
}

impl Default for ClockPlugin {
    fn default() -> Self {
        Self {
            server: DEFAULT_NTP_SERVER,
        }
    }
}

impl Plugin for ClockPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NtpServer(self.server));
        app.add_systems(PostStartup, spawn_clock);
    }
}

/// The configured NTP server, stashed for [`spawn_clock`].
#[derive(Resource, Clone, Copy)]
struct NtpServer(&'static str);

/// Spawn the SNTP loop. Exclusive so it can read the non-send [`Stack`] and
/// [`Spawner`]; runs in `PostStartup`, after Wi-Fi's `start_wifi`.
fn spawn_clock(world: &mut World) {
    let stack = *world.non_send::<Stack<'static>>();
    let spawner = *world.non_send::<Spawner>();
    let server = world.resource::<NtpServer>().0;
    spawn_driver(spawner, clock_loop(stack, server));
}

/// Wait for the network, then sync forever: install the `now` source on the
/// first success and re-poll on the interval, backing off on failure.
async fn clock_loop(stack: Stack<'static>, server: &'static str) {
    stack.wait_config_up().await;
    loop {
        match sync_once(stack, server).await {
            Ok(unix_millis) => {
                let uptime = Instant::now().as_millis() as i64;
                store_anchor(unix_millis as i64 - uptime);
                // First sync: hand the source to beet so `time_ext::now` works.
                if !SYNCED.swap(true, Ordering::Relaxed) {
                    if time_ext::set_now(sntp_now).is_err() {
                        warn!("a `now` clock source was already installed");
                    }
                    info!("SNTP synced; wall clock now available (unix_millis={})", unix_millis);
                } else {
                    info!("SNTP re-synced (unix_millis={})", unix_millis);
                }
                Timer::after(RESYNC_INTERVAL).await;
            }
            Err(e) => {
                warn!("SNTP sync failed: {}; retrying in 10s", e);
                Timer::after(RETRY_DELAY).await;
            }
        }
    }
}

/// One SNTP exchange: resolve the server, send a request, parse the transmit
/// timestamp, and return Unix time in milliseconds.
async fn sync_once(
    stack: Stack<'static>,
    server: &str,
) -> Result<u64, &'static str> {
    // `dns_query` short-circuits IP literals, so this covers both forms.
    let addrs = stack
        .dns_query(server, DnsQueryType::A)
        .await
        .map_err(|_| "DNS lookup for NTP server failed")?;
    let addr: IpAddress = *addrs.first().ok_or("no DNS results for NTP server")?;
    let remote = IpEndpoint::new(addr, 123);

    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_buf = [0u8; 128];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_buf = [0u8; 128];
    let mut socket =
        UdpSocket::new(stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    // Ephemeral local port; the stack picks one.
    socket.bind(0).map_err(|_| "UDP bind failed")?;

    // SNTP request: first byte packs LI=0, VN=4, Mode=3 (client). Rest zeroed.
    let mut request = [0u8; 48];
    request[0] = 0x23;
    socket
        .send_to(&request, remote)
        .await
        .map_err(|_| "UDP send failed")?;

    let mut response = [0u8; 48];
    let received = with_timeout(SNTP_TIMEOUT, socket.recv_from(&mut response)).await;
    let (n, _) = match received {
        Ok(Ok(pair)) => pair,
        Ok(Err(_)) => return Err("UDP receive error"),
        Err(_) => return Err("SNTP timed out"),
    };
    if n < 48 {
        return Err("short SNTP response");
    }

    // Transmit timestamp: 32-bit seconds at bytes 40..44, fraction at 44..48,
    // both big-endian, counted from the NTP epoch.
    let secs =
        u32::from_be_bytes([response[40], response[41], response[42], response[43]])
            as u64;
    let frac =
        u32::from_be_bytes([response[44], response[45], response[46], response[47]])
            as u64;
    if secs == 0 {
        return Err("invalid (zero) SNTP timestamp");
    }
    let unix_secs = secs
        .checked_sub(NTP_UNIX_OFFSET_SECS)
        .ok_or("SNTP timestamp before the Unix epoch")?;
    // fraction is a 2^32 fixed-point fraction of a second.
    let millis = unix_secs * 1000 + (frac * 1000 >> 32);
    Ok(millis)
}
