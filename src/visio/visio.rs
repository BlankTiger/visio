#![no_std]
#![no_main]
#![feature(never_type)]
#![feature(trait_alias)]

//! Binary for my engineering thesis project.
//!
//! Idea is to help visually impaired people in detecting obstacles in their surroundings via
//! haptic feedback. To accomplish this, vibration motors forming a matrix on users hand are formed
//! to provide a low resolution image through touch. Signal is generated by a Time of Flight (ToF)
//! sensor.
//!
//! To build this project I used:
//! - Arduino Nano RP2040 Connect
//! - VL53L1X ToF sensor
//! - Pca9685 PWM signal generator
//! - 16 vibration motors
//! - SSD1306 OLED display for debugging
//! - 3D printer to print cases for components
//! - a lot of patience..

use cortex_m::delay::Delay;
use embedded_graphics::mono_font::iso_8859_1::FONT_5X8;
use embedded_graphics::mono_font::MonoTextStyle;
use hal::i2c::Controller;
use hal::i2c::ValidPinScl;
use hal::i2c::ValidPinSda;
use pwm_pca9685::Channel;
use rp2040_hal::I2C;
use ssd1306::prelude::WriteOnlyDataCommand;
use ssd1306::I2CDisplayInterface;

use panic_halt as _;

use bsp::hal;
use bsp::hal::prelude::*;
use embedded_graphics::{
    mono_font::MonoTextStyleBuilder,
    pixelcolor::BinaryColor,
    prelude::*,
    text::{Baseline, Text},
};
use fugit::RateExtU32;
use pwm_pca9685::Address;
use pwm_pca9685::Pca9685;
use rp2040_hal::pac;
use ssd1306::{mode::BufferedGraphicsMode, prelude::*, Ssd1306};
use visiolib as bsp;
pub use vl53l1_reg::Index as Register;
use vl53l1x_uld::comm::Read;
use vl53l1x_uld::comm::Write;
use vl53l1x_uld::roi::{ROICenter, ROI};
use vl53l1x_uld::DistanceMode;
use vl53l1x_uld::IOVoltage;
use vl53l1x_uld::RangeStatus;
use vl53l1x_uld::VL53L1X;

extern crate alloc;
use embedded_alloc::Heap;

#[global_allocator]
static HEAP: Heap = Heap::empty();

/// Time of flight sensor SPAD centers ordered by index of vibration motor.
const TOF_CENTERS: [u8; 16] = [
    10, 42, 74, 106, // first row
    14, 46, 78, 110, // second row
    245, 213, 181, 149, // third row
    241, 209, 177, 145, // fourth row
];

#[visiolib::entry]
fn main() -> ! {
    init_heap();
    app().unwrap()
}

/// Heap initialization taken straight from embedded_alloc crate docs.
fn init_heap() {
    use core::mem::MaybeUninit;
    const HEAP_SIZE: usize = 1024;
    static mut HEAP_MEM: [MaybeUninit<u8>; HEAP_SIZE] = [MaybeUninit::uninit(); HEAP_SIZE];
    unsafe { HEAP.init(HEAP_MEM.as_ptr() as usize, HEAP_SIZE) }
}

/// Main program logic.
fn app() -> anyhow::Result<!> {
    let mut pac = pac::Peripherals::take().unwrap();
    let core = pac::CorePeripherals::take().unwrap();

    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        bsp::XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut delay = cortex_m::delay::Delay::new(core.SYST, clocks.system_clock.freq().to_Hz());

    let sio = hal::Sio::new(pac.SIO);

    let pins = bsp::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let i2c0 = I2C::i2c0(
        pac.I2C0,
        pins.a2.into_function(), // sda
        pins.a3.into_function(), // scl
        400.kHz(),
        &mut pac.RESETS,
        125_000_000.Hz(),
    );

    let i2c1 = I2C::i2c1(
        pac.I2C1,
        pins.a0.into_function(), // sda
        pins.a1.into_function(), // scl
        400.kHz(),
        &mut pac.RESETS,
        125_000_000.Hz(),
    );
    let shared_i2c1 = shared_bus::BusManagerSimple::new(i2c1);

    let i2c_display_interface = I2CDisplayInterface::new(shared_i2c1.acquire_i2c());
    let mut display = Ssd1306::new(
        i2c_display_interface,
        DisplaySize128x64,
        DisplayRotation::Rotate0,
    )
    .into_buffered_graphics_mode();
    display.init().unwrap();
    display.clear();
    display.flush().unwrap();

    let text_style = MonoTextStyleBuilder::new()
        .font(&FONT_5X8)
        .text_color(BinaryColor::On)
        .build();

    let small_style = MonoTextStyleBuilder::new()
        .font(&FONT_5X8)
        .text_color(BinaryColor::On)
        .build();

    let _error_style = MonoTextStyleBuilder::new()
        .font(&FONT_5X8)
        .text_color(BinaryColor::On)
        .build();

    display_text(&"Display initialized!", text_style, &mut display);
    delay.delay_ms(50);

    display_text(&"Pca9685 init...", text_style, &mut display);
    delay.delay_ms(50);

    let mut pwm_controller = Pca9685::new(shared_i2c1.acquire_i2c(), Address::default()).unwrap();
    pwm_controller.set_prescale(100).unwrap();
    pwm_controller.enable().unwrap();

    display_text(&"TOF init...", text_style, &mut display);
    delay.delay_ms(50);

    let mut tof = VL53L1X::new(i2c0, vl53l1x_uld::DEFAULT_ADDRESS);
    tof.init(IOVoltage::Volt2_8).unwrap();
    tof.set_distance_mode(DistanceMode::Short).unwrap();

    let _unknown_status_delay = 1500;
    let _verbose = false;

    if let Err(err) = tof.set_timing_budget_ms(33) {
        show_err(err, &mut display, &mut delay, small_style);
    }

    tof.set_inter_measurement_period_ms(35).unwrap();
    display_text(&"Ready!", text_style, &mut display);
    delay.delay_ms(50);
    display_text(&"Setting up TOF ROI..", text_style, &mut display);
    delay.delay_ms(50);

    // setting ROI
    let roi_size = ROI::new(4, 4);
    if let Err(e) = tof.set_roi(roi_size) {
        show_err(e, &mut display, &mut delay, small_style);
    }

    let mut distances = [0u16; 16];
    loop {
        update_distances(&mut tof, &mut distances);
        display_text(&stringify_distances(distances), small_style, &mut display);
        update_vibration_strength(&mut pwm_controller, distances);
    }
}

trait TOFI2C = Write<Error = rp2040_hal::i2c::Error> + Read<Error = rp2040_hal::i2c::Error>;

fn update_distances<T: TOFI2C>(tof: &mut VL53L1X<T>, distances: &mut [u16; 16]) {
    for (idx, center) in TOF_CENTERS.iter().enumerate() {
        tof.stop_ranging().unwrap();
        let roi_center = ROICenter { spad: *center };
        tof.set_roi_center(roi_center).unwrap();
        tof.start_ranging().unwrap();

        loop {
            match tof.get_range_status() {
                Ok(RangeStatus::Valid)
                | Ok(RangeStatus::SignalFailure)
                | Ok(RangeStatus::MinRangeFail) => {
                    tof.clear_interrupt().unwrap();
                    distances[idx] = tof.get_distance().unwrap();
                    break;
                }
                _ => {}
            }
        }
    }
}

type I2CInterface<Interface, Sda, Scl> = I2C<Interface, (Sda, Scl), Controller>;
type PwmController<I2C> = Pca9685<I2C>;
trait DerefToI2C = core::ops::Deref<Target = rp2040_hal::pac::i2c1::RegisterBlock>;

fn update_vibration_strength<
    Interface: DerefToI2C,
    Sda: ValidPinSda<Interface>,
    Scl: ValidPinScl<Interface>,
    Mutex: shared_bus::BusMutex<Bus = I2CInterface<Interface, Sda, Scl>>,
>(
    pwm_controller: &mut PwmController<shared_bus::I2cProxy<'_, Mutex>>,
    distances: [u16; 16],
) {
    for (idx, dist) in distances.iter().enumerate() {
        let channel = channel_from(idx as u8);
        let vibration_strength = vibration_strength_from(dist);
        pwm_controller
            .set_channel_on_off(channel, 0, vibration_strength)
            .unwrap();
    }
}

fn vibration_strength_from(dist: &u16) -> u16 {
    let mut strength = core::cmp::max(1300 - *dist, 0) * 2 / 3;
    if strength > 860 {
        strength = 0;
    }
    strength
}

fn channel_from(v: u8) -> Channel {
    match v {
        0 => Channel::C0,
        1 => Channel::C1,
        2 => Channel::C2,
        3 => Channel::C3,
        4 => Channel::C4,
        5 => Channel::C5,
        6 => Channel::C6,
        7 => Channel::C7,
        8 => Channel::C8,
        9 => Channel::C9,
        10 => Channel::C10,
        11 => Channel::C11,
        12 => Channel::C12,
        13 => Channel::C13,
        14 => Channel::C14,
        15 => Channel::C15,
        _ => Channel::All,
    }
}

type Display<DI, SIZE> = Ssd1306<DI, SIZE, BufferedGraphicsMode<SIZE>>;

fn show_err<E: core::fmt::Debug, DI: WriteOnlyDataCommand, SIZE: DisplaySize>(
    err: E,
    display: &mut Display<DI, SIZE>,
    delay: &mut Delay,
    text_style: MonoTextStyle<BinaryColor>,
) {
    let msg = alloc::format!("{:?}", err);
    display_text(&msg, text_style, display);
    delay.delay_ms(5000);
}

fn display_text<DI: WriteOnlyDataCommand, SIZE: DisplaySize, T: core::fmt::Display>(
    text: &T,
    text_style: MonoTextStyle<BinaryColor>,
    display: &mut Display<DI, SIZE>,
) {
    display.clear();
    let shown_text = alloc::format!("{}", text);
    Text::with_baseline(&shown_text, Point::zero(), text_style, Baseline::Top)
        .draw(display)
        .unwrap();
    display.flush().unwrap();
}

fn stringify_distances(distances: [u16; 16]) -> alloc::string::String {
    let d = distances;
    alloc::format!(
        "{}  {}  {}  {}\n{}  {}  {}  {}\n{}  {}  {}  {}\n{}  {}  {}  {}",
        d[0],
        d[1],
        d[2],
        d[3],
        d[4],
        d[5],
        d[6],
        d[7],
        d[8],
        d[9],
        d[10],
        d[11],
        d[12],
        d[13],
        d[14],
        d[15]
    )
}
