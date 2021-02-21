#![no_main]
#![no_std]
#![allow(unused_imports)]

use ds323x::{Ds323x, Rtcc};
use embedded_hal::blocking::{
    delay::{DelayMs, DelayUs},
    i2c::{Read, Write},
};
use nrf52840_hal::{
    self as hal,
    clocks::LfOscConfiguration,
    gpio::{p0::Parts as P0Parts, p1::Parts as P1Parts, Level},
    pac::{Peripherals, SPIM0, SPIS1, TIMER2, UARTE0},
    ppi::{Parts as PpiParts, Ppi0},
    spim::{Frequency, Pins as SpimPins, Spim, MODE_0},
    spis::{Mode, Pins as SpisPins, Spis, Transfer},
    timer::{Instance as TimerInstance, Periodic, Timer},
    twim::{Frequency as TwimFreq, Instance as TwimInstance, Pins as TwimPins, Twim},
    uarte::{Baudrate, Parity, Pins},
};
use sensor_scd30::Scd30;
use shared_bus::BusManagerSimple;
use spark_ser7seg::{i2c::SevSegI2c, SevenSegInterface};

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

    let scl = gpio0.p0_11;
    let sda = gpio0.p0_12;

    let twim = Twim::new(
        board.TWIM0,
        TwimPins {
            scl: scl.into_floating_input().degrade(),
            sda: sda.into_floating_input().degrade(),
        },
        TwimFreq::K400,
    );

    let bus = BusManagerSimple::new(twim);

    let mut timer = Timer::new(board.TIMER0);

    let mut sevseg = SevSegI2c::new(bus.acquire_i2c(), None);
    let mut ds3231 = Ds323x::new_ds3231(bus.acquire_i2c());
    let mut scd30 = None;

    sevseg.set_cursor(0).unwrap();
    timer.delay_us(15u32);
    let num = 1234;
    {
        if num > 9999 {
            panic!()
        }

        sevseg.set_cursor(0).unwrap();

        timer.delay_us(15u32);

        let data: [u8; 4] = [
            (num / 1000) as u8,
            ((num % 1000) / 100) as u8,
            ((num % 100) / 10) as u8,
            (num % 10) as u8,
        ];

        sevseg.send(&data).unwrap();
    }
    // sevseg.set_num(num).unwrap();

    loop {
        if scd30.is_none() {
            match Scd30::new(bus.acquire_i2c()) {
                Ok(scd) => {
                    defmt::info!("Got scd!");
                    scd30 = Some(scd);
                }
                Err(e) => {
                    defmt::error!("no scd...");
                    let mut i2c = bus.acquire_i2c();
                    let command = [0xd1, 0x00u8];
                    let mut rd_buf = [0u8; 3];

                    let a = i2c.write(0x61, &command);
                    let b = i2c.read(0x61, &mut rd_buf).ok();

                    if a.is_ok() && b.is_some() {
                        defmt::warn!("maj: {:?}, min: {:?}, crc: {:?}", rd_buf[0], rd_buf[1], rd_buf[2]);
                    } else {
                        defmt::error!("SCD manual failed!");
                    }
                }
            }
        }

        defmt::info!("cycling nums...");
        for i in 0..10 {
            let nums = [i as u8; 4];
            sevseg.write_digits(&nums).ok();
            timer.delay_ms(100u32);
        }

        defmt::info!("Checking DS3231 data...");
        if let Ok(temp) = ds3231.get_temperature() {
            defmt::info!("temp: {:?}", temp);
            timer.delay_ms(10u32);

            let num = temp as u16;
            match sevseg.write_digits(&num2bytes(num)) {
                Ok(_) => {}
                Err(e) => defmt::error!("sevseg error"),
            }
            timer.delay_ms(1000u32);

            if let (Ok(hour), Ok(minute)) = (ds3231.get_hours(), ds3231.get_minutes()) {
                let hour = match hour {
                    ds323x::Hours::AM(am) => am,
                    ds323x::Hours::PM(pm) => pm + 12,
                    ds323x::Hours::H24(h24) => h24,
                };

                defmt::info!("{:?}:{:?}", hour, minute);
                match sevseg.write_digits(&num2bytes(((hour * 100) + minute) as u16)) {
                    Ok(_) => {}
                    Err(e) => defmt::error!("sevseg error"),
                }
                timer.delay_ms(1000u32);
            }

            if let Ok(year) = ds3231.get_year() {
                defmt::info!("Year: {:?}", year);
                match sevseg.write_digits(&num2bytes(year as u16)) {
                    Ok(_) => {}
                    Err(e) => defmt::error!("sevseg error"),
                }
                timer.delay_ms(1000u32);
            }
        } else {
            defmt::warn!("No DS3231...");
        }

        timer.delay_ms(100u32);

        if let Some(scd) = &mut scd30 {
            defmt::info!("Checking SCD...");
            if scd.data_ready().unwrap() {
                let meas = scd.read_data().unwrap();

                timer.delay_ms(100u32);

                defmt::info!("co2: {:?}", meas.co2);
                sevseg.write_digits(&num2bytes(meas.co2 as u16)).unwrap();
                timer.delay_ms(1000u32);

                defmt::info!("temp: {:?}", meas.temp);
                sevseg.write_digits(&num2bytes(meas.temp as u16)).ok();
                timer.delay_ms(1000u32);

                defmt::info!("rh: {:?}", meas.rh);
                sevseg.write_digits(&num2bytes(meas.rh as u16)).ok();
                timer.delay_ms(1000u32);
            } else {
                defmt::warn!("SCD data not ready...");
            }
        }
    }
}

fn num2bytes(num: u16) -> [u8; 4] {
    let data: [u8; 4] = [
        (num / 1000) as u8,
        ((num % 1000) / 100) as u8,
        ((num % 100) / 10) as u8,
        (num % 10) as u8,
    ];
    data
}

