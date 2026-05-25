//! Wi-Fi station client.
//!
//! Connects to the AP named by the `SSID`/`PASSWORD` env vars (sourced from
//! `.env` by `build.rs`), obtains an IP via DHCP, then repeatedly performs an
//! HTTP GET against Cloudflare's `1.1.1.1` anycast address to demonstrate
//! outbound internet connectivity.
//!
//! Run with: `cargo run --release --example wifi-client`

#![no_std]
#![no_main]

use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::IpAddress;
use embassy_net::IpEndpoint;
use embassy_net::Ipv4Address;
use embassy_net::Runner;
use embassy_net::StackResources;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::scan::ScanConfig;
use esp_radio::wifi::{Config, ControllerConfig, Interface, WifiController, sta::StationConfig};
use panic_rtt_target as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    rtt_target::rtt_init_defmt!();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    info!("Wi-Fi client starting; target SSID `{}`", SSID);

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );
    let (mut controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("failed to initialize Wi-Fi controller");

    // Scan first: confirms the radio works and shows what's reachable.
    // The ESP32-S3 is 2.4 GHz only, so 5 GHz APs won't appear here.
    match controller.scan_async(&ScanConfig::default().with_max(20)).await {
        Ok(aps) => {
            info!("scan found {} access point(s):", aps.len());
            for ap in &aps {
                info!(
                    "  ssid=`{}` ch={} rssi={} auth={:?}",
                    ap.ssid.as_str(),
                    ap.channel,
                    ap.signal_strength,
                    ap.auth_method
                );
            }
        }
        Err(e) => warn!("scan failed: {:?}", e),
    }

    let net_config = embassy_net::Config::dhcpv4(Default::default());
    let rng = Rng::new();
    let seed = ((rng.random() as u64) << 32) | rng.random() as u64;

    let (stack, runner) = embassy_net::new(
        interfaces.station,
        net_config,
        mk_static!(StackResources<4>, StackResources::<4>::new()),
        seed,
    );

    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());

    info!("waiting for DHCP lease...");
    stack.wait_config_up().await;
    if let Some(cfg) = stack.config_v4() {
        info!("IPv4 config: {}", cfg);
    }

    // Cloudflare DNS, reachable over HTTP on its anycast address - no DNS needed.
    let remote = IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::new(1, 1, 1, 1)), 80);

    loop {
        let mut rx_buffer = [0u8; 1536];
        let mut tx_buffer = [0u8; 1536];
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        info!("connecting to 1.1.1.1:80");
        if let Err(e) = socket.connect(remote).await {
            warn!("connect failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        let request = b"GET / HTTP/1.1\r\nHost: 1.1.1.1\r\nConnection: close\r\n\r\n";
        if let Err(e) = socket.write_all(request).await {
            warn!("write failed: {:?}", e);
            Timer::after(Duration::from_secs(5)).await;
            continue;
        }

        let mut buf = [0u8; 512];
        let mut total = 0usize;
        let mut logged_head = false;
        loop {
            match socket.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if !logged_head {
                        logged_head = true;
                        if let Ok(s) = core::str::from_utf8(&buf[..n]) {
                            info!("response head:\n{}", s);
                        }
                    }
                }
                Err(e) => {
                    warn!("read failed: {:?}", e);
                    break;
                }
            }
        }
        info!("response complete, {} bytes total", total);

        Timer::after(Duration::from_secs(15)).await;
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        match controller.connect_async().await {
            Ok(info) => {
                info!("Wi-Fi connected: {:?}", info);
                let reason = controller.wait_for_disconnect_async().await;
                warn!("Wi-Fi disconnected: {:?}", reason);
            }
            Err(e) => {
                warn!("connect_async failed: {:?}; retrying in 3s", e);
                Timer::after(Duration::from_secs(3)).await;
            }
        }
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) -> ! {
    runner.run().await
}
