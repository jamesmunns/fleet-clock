#![no_main]
#![no_std]
#![allow(unused_imports)]

use ds323x::{Ds323x, NaiveDate, NaiveDateTime, NaiveTime, Rtcc};
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
    wdt::{count::One as OneDog, Watchdog},
};
use sensor_scd30::Scd30;
use shared_bus::BusManagerSimple;
use spark_ser7seg::{i2c::SevSegI2c, PunctuationFlags, SevenSegInterface};

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

    // Obtain the watchdog, or try to recover it (if already
    // active/running), or just spin and wait for the dog to bite.
    let mut wdh = Watchdog::try_new(board.WDT)
        .map(|mut wdt| {
            wdt.set_lfosc_ticks(32768 * 15);
            wdt.activate::<OneDog>()
        })
        .or_else(Watchdog::try_recover::<OneDog>)
        .unwrap_or_else(|_| loop {
            cortex_m::asm::wfi()
        })
        .handles
        .0;

    // Good doggy.
    wdh.pet();

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
    let mut scd30 = Scd30::new(bus.acquire_i2c()).unwrap();

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
    timer.delay_us(15u32);

    sevseg.write_digits(&time2bytes(hours, mins)).unwrap();
    timer.delay_us(100u32);
    sevseg
        .write_punctuation(PunctuationFlags::DOTS_COLON)
        .unwrap();

    let mut min_uptime = 0u32;

    loop {
        let new_hours = ds3231.get_hours().unwrap();
        let new_mins = ds3231.get_minutes().unwrap();
        let new_secs = ds3231.get_seconds().unwrap();

        // Pet the dog.
        wdh.pet();

        // TODO: End of hour report?

        if mins != new_mins {
            min_uptime += 1;

            defmt::info!("Checking SCD...");
            if scd30.data_ready().unwrap() {
                sevseg.set_cursor(0).unwrap();
                timer.delay_us(100u32);
                sevseg.write_punctuation(PunctuationFlags::NONE).unwrap();
                timer.delay_us(100u32);

                sevseg.send(b" co2").unwrap();
                timer.delay_ms(2000u32);

                let meas = scd30.read_data().unwrap();
                defmt::info!("co2: {:?}", meas.co2);
                sevseg.write_digits(&num2bytes(meas.co2 as u16)).unwrap();
                timer.delay_ms(2000u32);

                defmt::info!("temp: {:?}", meas.temp);
                sevseg
                    .write_digits(&num2bytes((meas.temp * 100.0) as u16))
                    .ok();
                timer.delay_us(100u32);
                sevseg
                    .write_punctuation(PunctuationFlags::DOT_BETWEEN_2_AND_3)
                    .unwrap();
                timer.delay_us(100u32);
                sevseg.set_cursor(3).unwrap();
                timer.delay_us(100u32);
                sevseg.send(b"C").unwrap();
                timer.delay_ms(2000u32);

                sevseg.write_punctuation(PunctuationFlags::NONE).unwrap();
                timer.delay_us(100u32);

                defmt::info!("rh: {:?}", meas.rh);
                sevseg
                    .write_digits(&num2bytes((meas.rh * 100.0) as u16))
                    .ok();
                timer.delay_us(100u32);
                sevseg.set_cursor(2).unwrap();
                timer.delay_us(100u32);
                sevseg.send(b"rh").unwrap();
                timer.delay_ms(2000u32);

                defmt::info!("uptime_mins: {:?}", min_uptime);

                let hours_up = (min_uptime as f32) / 60.0;
                let days_up = (min_uptime as f32) / 1440.0;

                let updata = if hours_up <= 99.9f32 {
                    let show = (hours_up * 100f32) as u16;
                    let dot = PunctuationFlags::DOT_BETWEEN_1_AND_2;
                    let unit = b"h";
                    Some((show, dot, unit))
                } else if hours_up <= 999.5f32 {
                    let show = (hours_up * 10f32) as u16;
                    let dot = PunctuationFlags::DOT_BETWEEN_2_AND_3;
                    let unit = b"h";
                    Some((show, dot, unit))
                } else if days_up <= 99.9f32 {
                    let show = (days_up * 100f32) as u16;
                    let dot = PunctuationFlags::DOT_BETWEEN_1_AND_2;
                    let unit = b"d";
                    Some((show, dot, unit))
                } else if days_up <= 999.5f32 {
                    let show = (days_up * 10f32) as u16;
                    let dot = PunctuationFlags::DOT_BETWEEN_2_AND_3;
                    let unit = b"d";
                    Some((show, dot, unit))
                } else if days_up > 999.5f32 {
                    None
                } else {
                    defmt::warn!("Floats suck.");
                    None
                };

                if let Some((show, dot, unit)) = updata {
                    sevseg.write_digits(&num2bytes(show)).ok();
                    timer.delay_us(100u32);
                    sevseg.write_punctuation(dot).unwrap();
                    timer.delay_us(100u32);
                    sevseg.set_cursor(3).unwrap();
                    timer.delay_us(100u32);
                    sevseg.send(unit).unwrap();
                    timer.delay_ms(2000u32);
                    sevseg.write_punctuation(PunctuationFlags::NONE).unwrap();
                    timer.delay_us(100u32);
                }
            }
        } else if new_secs != secs {
            let all_dots = PunctuationFlags::DOT_BETWEEN_1_AND_2
                | PunctuationFlags::DOT_BETWEEN_2_AND_3
                | PunctuationFlags::DOT_BETWEEN_3_AND_4
                | PunctuationFlags::DOT_RIGHT_OF_4;

            let punc = all_dots
                ^ if secs < 15 {
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
                sevseg
                    .write_punctuation(PunctuationFlags::DOTS_COLON | punc)
                    .unwrap();
            } else {
                sevseg
                    .write_punctuation(PunctuationFlags::NONE | punc)
                    .unwrap();
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
