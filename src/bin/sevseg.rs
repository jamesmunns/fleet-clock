#![no_main]
#![no_std]
#![allow(unused_imports)]

use embedded_hal::blocking::delay::DelayMs;
use nrf52840_hal::{
    self as hal,
    clocks::LfOscConfiguration,
    gpio::{p0::Parts as P0Parts, p1::Parts as P1Parts, Level},
    pac::{Peripherals, SPIM0, SPIS1, TIMER2, UARTE0},
    ppi::{Parts as PpiParts, Ppi0},
    spim::{Frequency, Pins as SpimPins, Spim, MODE_0},
    spis::{Mode, Pins as SpisPins, Spis, Transfer},
    twim::{Instance as TwimInstance, Pins as TwimPins, Frequency as TwimFreq, Twim},
    timer::{Instance as TimerInstance, Periodic, Timer},
    uarte::{Baudrate, Parity, Pins},
};
use spark_ser7seg::{
    i2c::SevSegI2c,
    SevenSegInterface,
};
use shared_bus::BusManagerSimple;
use sensor_scd30::Scd30;
use ds323x::Ds323x;

use fleet_clock as _; // global logger + panicking-behavior + memory layout

#[cortex_m_rt::entry]
fn main() -> ! {
    defmt::info!("Hello, world!");

    let board = Peripherals::take().unwrap();

    // Setup clocks
    let clocks = hal::clocks::Clocks::new(board.CLOCK);
    let clocks = clocks.enable_ext_hfosc();
    let clocks = clocks.set_lfclk_src_external(LfOscConfiguration::NoExternalNoBypass);
    clocks.start_lfclk();

    let gpio0 = P0Parts::new(board.P0);
    let _gpio1 = P1Parts::new(board.P1);

    let scl = gpio0.p0_11.into_floating_input().degrade();
    let sda = gpio0.p0_12.into_floating_input().degrade();

    let twim = Twim::new(
        board.TWIM0,
        TwimPins {
            scl,
            sda,
        },
        TwimFreq::K400
    );

    // let bus = BusManagerSimple::new(twim);

    let mut timer = Timer::new(board.TIMER0);

    // let mut ds3231 = Ds323x::new_ds3231(bus.acquire_i2c());
    // let mut sevseg = SevSegI2c::new(bus.acquire_i2c(), None);
    let mut sevseg = SevSegI2c::new(twim, None);

    // let mut scd30: Option<_> = None;


    // let mut ds3231 = Ds323x::new_ds3231(twim);

    // let fwv = scd30.firmware_version().unwrap();
    // defmt::info!("SCD Firmware Version: {:?}", fwv);

    // scd30.start_continuous(0).unwrap();

    loop {
        // if scd30.is_none() {
        //     match Scd30::new(bus.acquire_i2c()) {
        //         Ok(scd) => {
        //             defmt::info!("Got scd!");
        //             scd30 = Some(scd);
        //         }
        //         Err(e) => {
        //             defmt::error!("no scd...");
        //         }
        //     }
        // }

        // if let Some(scd) = &mut scd30 {
        //     defmt::info!("Checking SCD...");
        //     if scd.data_ready().unwrap() {
        //         let meas = scd.read_data().unwrap();

        //         timer.delay_ms(100u32);

        //         defmt::info!("co2: {:?}", meas.co2);
        //         sevseg.set_num(meas.co2 as u16).unwrap();
        //         timer.delay_ms(1000u32);

        //         defmt::info!("temp: {:?}", meas.temp);
        //         sevseg.set_num(meas.temp as u16).ok();
        //         timer.delay_ms(1000u32);

        //         defmt::info!("rh: {:?}", meas.rh);
        //         sevseg.set_num(meas.rh as u16).ok();
        //         timer.delay_ms(1000u32);
        //     } else {
        //         defmt::warn!("SCD data not ready...");
        //     }
        // }

        timer.delay_ms(100u32);

        // defmt::info!("Checking DS3231 data...");
        // if let Ok(temp) = ds3231.get_temperature() {
        //     defmt::info!("temp: {:?}", temp);
        //     timer.delay_ms(100u32);
        //     sevseg.set_num(temp as u16).unwrap();
        //     timer.delay_ms(1000u32);
        // } else {
        //     defmt::warn!("No DS3231...");
        // }

        defmt::info!("i sleep.");
        sevseg.set_num(8888u16).unwrap();
        timer.delay_ms(1000u32);
    }
}
