//! Wi-Fi TCP server.
//!
//! Joins the AP named by `SSID`/`PASSWORD` (from `.env`), gets a DHCP lease,
//! then serves a tiny HTTP page on port 8080. Each request bumps a counter.
//!
//! Once it logs its IP, hit it from a machine on the same LAN:
//!   curl http://<device-ip>:8080
//!
//! Run with: `cargo run --release --example wifi-server`

#![no_std]
#![no_main]

use core::fmt::Write as _;

use defmt::info;
use defmt::warn;
use embassy_executor::Spawner;
use embassy_net::Runner;
use embassy_net::StackResources;
use embassy_net::tcp::TcpSocket;
use embassy_time::{Duration, Timer};
use embedded_io_async::Write as _;
use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::wifi::{Config, ControllerConfig, Interface, WifiController, sta::StationConfig};
use panic_rtt_target as _;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");
const PORT: u16 = 8080;

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        #[deny(unused_attributes)]
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

/// Minimal `core::fmt::Write` sink over a fixed byte buffer.
struct ByteBuf<'a> {
    buf: &'a mut [u8],
    len: usize,
}

impl core::fmt::Write for ByteBuf<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let end = self.len + bytes.len();
        if end > self.buf.len() {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..end].copy_from_slice(bytes);
        self.len = end;
        Ok(())
    }
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

    info!("Wi-Fi server starting; joining SSID `{}`", SSID);

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );
    let (controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .expect("failed to initialize Wi-Fi controller");

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
        info!("ready: serving http://{}:{}", cfg.address.address(), PORT);
    }

    let mut count: u32 = 0;
    loop {
        let mut rx_buffer = [0u8; 1536];
        let mut tx_buffer = [0u8; 1536];
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(Duration::from_secs(10)));

        info!("listening on :{}", PORT);
        if let Err(e) = socket.accept(PORT).await {
            warn!("accept failed: {:?}", e);
            continue;
        }
        count += 1;
        info!("connection #{} from {:?}", count, socket.remote_endpoint());

        // Drain the request (we don't parse it, just respond).
        let mut req = [0u8; 512];
        let _ = socket.read(&mut req).await;

        let mut resp = [0u8; 256];
        let mut w = ByteBuf {
            buf: &mut resp,
            len: 0,
        };
        let _ = write!(
            w,
            "HTTP/1.0 200 OK\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\n\
             Hello from ESP32-S3 (esp-hal + embassy)!\r\nRequest #{count}\r\n"
        );
        let n = w.len;

        if let Err(e) = socket.write_all(&resp[..n]).await {
            warn!("write failed: {:?}", e);
        }
        // flush() waits until the response has been sent, then close() sends the
        // FIN. We re-accept immediately so there's no gap where the port is closed.
        if let Err(e) = socket.flush().await {
            warn!("flush failed: {:?}", e);
        }
        socket.close();
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
