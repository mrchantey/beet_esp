//! BLE central ("client") scanner using trouBLE.
//!
//! Brings up the BLE controller and passively/actively scans for nearby BLE
//! advertisements, logging each newly-seen device's address, kind and RSSI.
//! No second device is required - there are almost always BLE beacons around
//! (phones, headphones, watches, etc.).
//!
//! Run with: `cargo run --release --example bluetooth-client`

#![no_std]
#![no_main]

use core::cell::RefCell;

use bt_hci::controller::ExternalController;
use bt_hci::param::LeAdvReportsIter;
use embassy_executor::Spawner;
use embassy_futures::join::join;
use log::info;
use embassy_time::{Duration, Timer};
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::ble::controller::BleConnector;
use beet_esp::prelude::init_rtt_log;
use trouble_host::prelude::*;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 1;
const MAX_SEEN: usize = 64;

#[esp_rtos::main]
async fn main(_spawner: Spawner) -> ! {
    init_rtt_log();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);
    esp_alloc::heap_allocator!(size: 64 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_interrupt =
        esp_hal::interrupt::software::SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_interrupt.software_interrupt0);

    let connector = BleConnector::new(peripherals.BT, Default::default()).unwrap();
    let controller: ExternalController<_, 1> = ExternalController::new(connector);

    // A fixed random address is fine for a scanner.
    let address = Address::random([0xff, 0x8f, 0x1b, 0x05, 0xe4, 0xff]);
    info!("BLE scanner starting; our address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        central,
        mut runner,
        ..
    } = stack.build();

    let printer = Printer {
        seen: RefCell::new(SeenList::new()),
    };
    let mut scanner = Scanner::new(central);

    // The host runner must be polled continuously; run it alongside the scan.
    let _ = join(runner.run_with_handler(&printer), async {
        let _session = scanner.scan(&ScanConfig::default()).await.unwrap();
        info!("scanning for BLE advertisements...");
        loop {
            Timer::after(Duration::from_secs(5)).await;
            info!(
                "still scanning; {} unique device(s) seen",
                printer.seen.borrow().count
            );
        }
    })
    .await;

    // `join` only returns if the host runner exits (an error); keep the entry
    // point divergent regardless.
    loop {
        Timer::after(Duration::from_secs(1)).await;
    }
}

struct SeenList {
    addrs: [[u8; 6]; MAX_SEEN],
    count: usize,
}

impl SeenList {
    const fn new() -> Self {
        Self {
            addrs: [[0u8; 6]; MAX_SEEN],
            count: 0,
        }
    }

    /// Records `addr`; returns true if it had not been seen before.
    fn insert(&mut self, addr: [u8; 6]) -> bool {
        if self.addrs[..self.count].contains(&addr) {
            return false;
        }
        if self.count < MAX_SEEN {
            self.addrs[self.count] = addr;
            self.count += 1;
        }
        true
    }
}

struct Printer {
    seen: RefCell<SeenList>,
}

impl EventHandler for Printer {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        let mut seen = self.seen.borrow_mut();
        while let Some(Ok(report)) = it.next() {
            let mut addr = [0u8; 6];
            addr.copy_from_slice(report.addr.raw());
            if seen.insert(addr) {
                info!(
                    "discovered {:?} (kind={:?}, rssi={})",
                    report.addr, report.addr_kind, report.rssi
                );
            }
        }
    }
}
