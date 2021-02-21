#![no_main]
#![no_std]
#![allow(unused_imports)]

use ds323x::{Ds323x, Rtcc, NaiveDateTime, NaiveDate, NaiveTime};
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
use spark_ser7seg::{i2c::SevSegI2c, SevenSegInterface, PunctuationFlags};

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

    let date = NaiveDate::from_ymd(2021, 2, 21);
    let time = NaiveTime::from_hms(22, 36, 40);
    let base_now = NaiveDateTime::new(date, time);

    let dt = ds3231.get_datetime().unwrap();

    let mut time_sep = true;

    // HACK FOR TIME INIT
    if dt < base_now {
        defmt::warn!("Setting clock!");
        ds3231.set_datetime(&base_now).unwrap();
        timer.delay_ms(10u32);
    }

    let mut hours = ds3231.get_hours().unwrap();
    let mut mins = ds3231.get_minutes().unwrap();
    let mut secs = ds3231.get_seconds().unwrap();


    sevseg.set_cursor(0).unwrap();
    timer.delay_us(100u32);
    sevseg.write_punctuation(PunctuationFlags::NONE).unwrap();
    timer.delay_us(100u32);
    let num = 1234;
    {
        if num > 9999 {
            panic!()
        }

        sevseg.set_cursor(0).unwrap();

        timer.delay_us(100u32);

        let data: [u8; 4] = [
            (num / 1000) as u8,
            ((num % 1000) / 100) as u8,
            ((num % 100) / 10) as u8,
            (num % 10) as u8,
        ];

        sevseg.send(&data).unwrap();
        timer.delay_ms(1000u32);
    }

    defmt::info!("cycling nums...");
    for i in 0..16 {
        let nums = [i as u8; 4];
        sevseg.write_digits(&nums).ok();
        timer.delay_ms(100u32);
    }




    sevseg.set_cursor(0).unwrap();
    timer.delay_us(15u32);

    sevseg.write_digits(&time2bytes(hours, mins)).unwrap();
    timer.delay_us(100u32);
    sevseg.write_punctuation(PunctuationFlags::DOTS_COLON).unwrap();

    loop {
        let new_hours = ds3231.get_hours().unwrap();
        let new_mins = ds3231.get_minutes().unwrap();
        let new_secs = ds3231.get_seconds().unwrap();

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


        // TODO: End of hour report?

        if scd30.is_some() && (mins != new_mins) {
            if let Some(scd) = &mut scd30 {
                defmt::info!("Checking SCD...");
                if scd.data_ready().unwrap() {

                    sevseg.set_cursor(0).unwrap();
                    timer.delay_us(100u32);
                    sevseg.write_punctuation(PunctuationFlags::NONE).unwrap();
                    timer.delay_us(100u32);

                    sevseg.send(b" co2").unwrap();
                    timer.delay_ms(2000u32);

                    let meas = scd.read_data().unwrap();
                    defmt::info!("co2: {:?}", meas.co2);
                    sevseg.write_digits(&num2bytes(meas.co2 as u16)).unwrap();
                    timer.delay_ms(2000u32);

                    defmt::info!("temp: {:?}", meas.temp);
                    sevseg.write_digits(&num2bytes((meas.temp * 100.0) as u16)).ok();
                    timer.delay_us(100u32);
                    sevseg.write_punctuation(PunctuationFlags::DOT_BETWEEN_2_AND_3).unwrap();
                    timer.delay_us(100u32);
                    sevseg.set_cursor(3).unwrap();
                    timer.delay_us(100u32);
                    sevseg.send(b"C").unwrap();
                    timer.delay_ms(2000u32);

                    sevseg.write_punctuation(PunctuationFlags::NONE).unwrap();
                    timer.delay_us(100u32);

                    defmt::info!("rh: {:?}", meas.rh);
                    sevseg.write_digits(&num2bytes((meas.rh * 100.0) as u16)).ok();
                    timer.delay_us(100u32);
                    sevseg.set_cursor(2).unwrap();
                    timer.delay_us(100u32);
                    sevseg.send(b"rh").unwrap();
                    timer.delay_ms(2000u32);

                } else {
                    defmt::warn!("SCD data not ready...");
                }
            }
        } else if new_secs != secs {
            let punc = if secs < 15 {
                PunctuationFlags::DOT_BETWEEN_1_AND_2
            } else if secs < 30 {
                PunctuationFlags::DOT_BETWEEN_2_AND_3
            } else if secs < 45 {
                PunctuationFlags::DOT_BETWEEN_3_AND_4
            } else {
                PunctuationFlags::DOT_RIGHT_OF_4
            };



            time_sep = !time_sep;
            if time_sep {
                sevseg.write_punctuation(PunctuationFlags::DOTS_COLON | punc).unwrap();
            } else {
                sevseg.write_punctuation(PunctuationFlags::NONE | punc).unwrap();
            }
            timer.delay_us(100u32);
        }

        hours = new_hours;
        mins = new_mins;
        secs = new_secs;

        sevseg.write_digits(&time2bytes(hours, mins)).unwrap();

        timer.delay_ms(100u32);
    }
}

fn time2bytes(h: ds323x::Hours, m: u8) -> [u8; 4] {
    let hour = match h {
        ds323x::Hours::AM(am) => am,
        ds323x::Hours::PM(pm) => pm + 12,
        ds323x::Hours::H24(h24) => h24,
    };

    num2bytes(((hour as u32 * 100) as u16 + m as u16) as u16)
}

fn num2bytes(mut num: u16) -> [u8; 4] {

    num = num.min(9999);

    let data: [u8; 4] = [
        (num / 1000) as u8,
        ((num % 1000) / 100) as u8,
        ((num % 100) / 10) as u8,
        (num % 10) as u8,
    ];
    data
}

