//! The embassy UART driver: owns UART1 and the STM32 reset/check GPIOs, runs the
//! bring-up handshake, then pumps [`Command`]s out and [`Status`] in.
//!
//! Continuous sensor state is coalesced into a [`StatusSnapshot`] published on
//! [`ALVIK_STATE`] (latest-wins — the ECS reads the newest each frame), while
//! discrete messages (acks, touch, move, behaviour, firmware) go through the
//! lossless [`ALVIK_EVENTS`] queue. Commands queued by systems on [`ALVIK_OUT`]
//! are drained and written to the wire.

use crate::alvik::pinout;
use crate::alvik::protocol::Command;
use crate::alvik::protocol::Status;
use crate::alvik::ucpack::Decoder;
use crate::alvik::ucpack::Encoder;
use crate::esp32_utils::async_bridge::Latest;
use crate::esp32_utils::async_bridge::Queue;
use crate::esp32_utils::async_bridge::spawn_driver;
use crate::utils::units::Angle;
use crate::utils::units::AngularVelocity;
use crate::utils::units::Distance;
use crate::utils::units::LinearVelocity;
use beet::prelude::*;
use embassy_executor::Spawner;
use embassy_futures::select::Either;
use embassy_futures::select::select;
use embassy_time::Duration;
use embassy_time::Instant;
use embassy_time::Timer;
use embassy_time::with_timeout;
use esp_hal::Async;
use esp_hal::gpio::Input;
use esp_hal::gpio::InputConfig;
use esp_hal::gpio::Level;
use esp_hal::gpio::Output;
use esp_hal::gpio::OutputConfig;
use esp_hal::gpio::Pull;
use esp_hal::peripherals::GPIO5;
use esp_hal::peripherals::GPIO6;
use esp_hal::peripherals::GPIO7;
use esp_hal::peripherals::GPIO13;
use esp_hal::peripherals::GPIO43;
use esp_hal::peripherals::GPIO44;
use esp_hal::peripherals::UART1;
use esp_hal::uart::Config;
use esp_hal::uart::RxError;
use esp_hal::uart::Uart;

/// The firmware version this port targets (`__required_firmware_version__`).
/// A mismatch is warned over defmt and otherwise ignored (never panics).
const REQUIRED_FW: (u8, u8, u8) = (1, 1, 1);

/// Commands queued by ECS systems, drained and written by the driver.
pub static ALVIK_OUT: Queue<Command, 16> = Queue::new();
/// Latest coalesced continuous sensor state, applied to components each frame.
pub static ALVIK_STATE: Latest<StatusSnapshot> = Latest::new();
/// Discrete status messages (acks, touch, move, behaviour, firmware).
pub static ALVIK_EVENTS: Queue<Status, 32> = Queue::new();

/// Coalesced snapshot of the continuous sensor state. The driver folds each
/// streaming sensor message into one of these and publishes the whole snapshot
/// on [`ALVIK_STATE`]; discrete messages are returned for the event queue.
#[derive(Clone, Copy, Default)]
pub struct StatusSnapshot {
    pub wheel_left_speed: AngularVelocity,
    pub wheel_right_speed: AngularVelocity,
    pub wheel_left_position: Angle,
    pub wheel_right_position: Angle,
    pub line: (i16, i16, i16),
    pub color: (i16, i16, i16),
    pub imu: [f32; 6],
    pub orientation: (Angle, Angle, Angle),
    pub battery: (f32, bool),
    pub tof_left: Distance,
    pub tof_center_left: Distance,
    pub tof_center: Distance,
    pub tof_center_right: Distance,
    pub tof_right: Distance,
    pub tof_top: Distance,
    pub tof_bottom: Distance,
    pub velocity: (LinearVelocity, AngularVelocity),
    pub pose: (Distance, Distance, Angle),
}

impl StatusSnapshot {
    /// Fold a continuous sensor `status` into the snapshot; return discrete
    /// messages unchanged for the caller to enqueue.
    fn apply(&mut self, status: Status) -> Option<Status> {
        match status {
            Status::WheelSpeeds { left, right } => {
                self.wheel_left_speed = left;
                self.wheel_right_speed = right;
            }
            Status::WheelPositions { left, right } => {
                self.wheel_left_position = left;
                self.wheel_right_position = right;
            }
            Status::Line {
                left,
                center,
                right,
            } => self.line = (left, center, right),
            Status::Color { red, green, blue } => self.color = (red, green, blue),
            Status::Imu(imu) => self.imu = imu,
            Status::Orientation { roll, pitch, yaw } => self.orientation = (roll, pitch, yaw),
            Status::Battery { percent, charging } => self.battery = (percent, charging),
            Status::Tof {
                left,
                center,
                right,
            } => {
                self.tof_left = left;
                self.tof_center = center;
                self.tof_right = right;
            }
            Status::TofMatrix {
                left,
                center_left,
                center,
                center_right,
                right,
                top,
                bottom,
            } => {
                self.tof_left = left;
                self.tof_center_left = center_left;
                self.tof_center = center;
                self.tof_center_right = center_right;
                self.tof_right = right;
                self.tof_top = top;
                self.tof_bottom = bottom;
            }
            Status::Velocity { linear, angular } => self.velocity = (linear, angular),
            Status::Pose { x, y, theta } => self.pose = (x, y, theta),
            discrete => return Some(discrete),
        }
        None
    }
}

/// Owns UART1 and the STM32 control/check GPIOs, plus the ucPack codec state.
struct AlvikDriver {
    uart: Uart<'static, Async>,
    reset: Output<'static>,
    /// Held low for normal STM32 boot (kept alive so the level persists).
    _boot0: Output<'static>,
    /// Held low outside idle mode (kept alive so the level persists).
    _nano_chk: Output<'static>,
    check_stm32: Input<'static>,
    encoder: Encoder,
    decoder: Decoder,
}

impl AlvikDriver {
    /// Run the bring-up handshake, then pump commands out and status in forever.
    async fn run(mut self) {
        self.bring_up().await;
        let mut snapshot = StatusSnapshot::default();
        loop {
            let mut buf = [0u8; 128];
            let command = match select(self.uart.read_async(&mut buf), ALVIK_OUT.recv()).await {
                Either::First(Ok(read)) => {
                    self.decoder.extend(&buf[..read]);
                    while let Some(frame) = self.decoder.pop() {
                        if let Some(status) = Status::decode(&frame) {
                            match snapshot.apply(status) {
                                None => ALVIK_STATE.send(snapshot),
                                Some(discrete) => {
                                    let _ = ALVIK_EVENTS.send(discrete);
                                }
                            }
                        }
                    }
                    None
                }
                Either::First(Err(e)) => {
                    // A full hardware RX FIFO while we were mid-write is expected
                    // under the Alvik's continuous sensor stream: ucPack reframes
                    // on the next valid frame and continuous state is latest-wins,
                    // so the dropped bytes are harmless. Log it quietly and only
                    // warn on the genuine line errors.
                    if e == RxError::FifoOverflowed {
                        defmt::debug!("alvik rx fifo overflow (recovered)");
                    } else {
                        defmt::warn!("alvik rx error: {}", e);
                    }
                    None
                }
                Either::Second(command) => Some(command),
            };
            if let Some(command) = command {
                self.send(command).await;
            }
        }
    }

    /// The `begin()` handshake: wait for power, reset the STM32, await the ack +
    /// firmware version (warn-only on mismatch), then send the init commands.
    async fn bring_up(&mut self) {
        defmt::info!("alvik: waiting for robot power (CHECK_STM32 high)...");
        while self.check_stm32.is_low() {
            Timer::after(Duration::from_millis(100)).await;
        }
        defmt::info!("alvik: robot on — resetting STM32");
        self.reset.set_low();
        Timer::after(Duration::from_millis(100)).await;
        self.reset.set_high();
        Timer::after(Duration::from_millis(100)).await;
        self.flush_uart();

        // Await the 0x00 ack and the firmware version, up to 5s.
        let mut got_ack = false;
        let mut firmware: Option<(u8, u8, u8)> = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = [0u8; 128];
        while !(got_ack && firmware.is_some()) && Instant::now() < deadline {
            if let Ok(Ok(read)) =
                with_timeout(Duration::from_millis(500), self.uart.read_async(&mut buf)).await
            {
                self.decoder.extend(&buf[..read]);
                while let Some(frame) = self.decoder.pop() {
                    match Status::decode(&frame) {
                        Some(Status::Ack(0)) => got_ack = true,
                        Some(Status::FwVersion {
                            major,
                            minor,
                            patch,
                        }) => firmware = Some((major, minor, patch)),
                        _ => {}
                    }
                }
            }
        }

        match firmware {
            Some(version) if version == REQUIRED_FW => {
                defmt::info!("alvik: firmware {} OK", version)
            }
            Some(version) => defmt::warn!(
                "alvik: firmware {} != required {} — continuing anyway",
                version,
                REQUIRED_FW
            ),
            None => defmt::warn!("alvik: no firmware version received — continuing"),
        }
        if !got_ack {
            defmt::warn!("alvik: no startup ack received — continuing");
        }

        // Init sequence: illuminator on, behaviors 1 & 2, servos centred.
        self.send(Command::SetLeds(0b0000_0010)).await;
        self.send(Command::SetBehavior(1)).await;
        self.send(Command::SetBehavior(2)).await;
        self.send(Command::SetServo { a: 90, b: 90 }).await;

        // Tell the ECS the link is up (sets Connected + FirmwareVersion).
        if let Some((major, minor, patch)) = firmware {
            let _ = ALVIK_EVENTS.send(Status::FwVersion {
                major,
                minor,
                patch,
            });
        }
        defmt::info!("alvik: bring-up complete");
    }

    /// Encode `command` and write it to the UART.
    async fn send(&mut self, command: Command) {
        let bytes = command.encode(&mut self.encoder);
        if let Err(e) = self.uart.write_async(bytes).await {
            defmt::warn!("alvik tx error: {}", e);
        }
    }

    /// Drop any bytes sitting in the RX buffer and reset the frame decoder.
    fn flush_uart(&mut self) {
        let mut scratch = [0u8; 64];
        while let Ok(read) = self.uart.read_buffered(&mut scratch) {
            if read == 0 {
                break;
            }
        }
        self.decoder = Decoder::new();
    }
}

/// Claim UART1 + the Alvik GPIOs that [`bring_up`](crate::esp32_plugin) exposed,
/// build the [`AlvikDriver`], and spawn it. Exclusive so it can pull the
/// non-send peripherals and the [`Spawner`]; runs in `Startup`, mirroring
/// `spawn_led_driver`.
pub fn spawn_alvik_driver(world: &mut World) {
    let uart1 = world
        .remove_non_send::<UART1<'static>>()
        .expect("add Esp32Plugin before AlvikPlugin — bring_up provides UART1");
    let tx = world.remove_non_send::<GPIO43<'static>>().expect("GPIO43");
    let rx = world.remove_non_send::<GPIO44<'static>>().expect("GPIO44");
    let reset = world.remove_non_send::<GPIO6<'static>>().expect("GPIO6");
    let boot0 = world.remove_non_send::<GPIO5<'static>>().expect("GPIO5");
    let nano_chk = world.remove_non_send::<GPIO7<'static>>().expect("GPIO7");
    let check = world.remove_non_send::<GPIO13<'static>>().expect("GPIO13");
    let spawner = *world.non_send::<Spawner>();

    let uart = Uart::new(uart1, Config::default().with_baudrate(pinout::BAUD_RATE))
        .expect("failed to configure UART1")
        .with_tx(tx)
        .with_rx(rx)
        .into_async();

    let driver = AlvikDriver {
        uart,
        // RESET is active-low: idle high so the STM32 runs.
        reset: Output::new(reset, Level::High, OutputConfig::default()),
        _boot0: Output::new(boot0, Level::Low, OutputConfig::default()),
        _nano_chk: Output::new(nano_chk, Level::Low, OutputConfig::default()),
        check_stm32: Input::new(check, InputConfig::default().with_pull(Pull::Down)),
        encoder: Encoder::new(),
        decoder: Decoder::new(),
    };
    spawn_driver(spawner, driver.run());
}
