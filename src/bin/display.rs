#![no_main]
#![no_std]
#![allow(unused_imports, dead_code, unused_variables, unused_mut)]

use ds323x::{Ds323x, NaiveDate, NaiveDateTime, NaiveTime, Rtcc};
use embedded_hal::blocking::{
    delay::{DelayMs, DelayUs},
    i2c::{Read, Write},
    spi::Write as SpimWrite,
};
use hal::prelude::OutputPin;
use nrf52840_hal::{
    self as hal,
    clocks::LfOscConfiguration,
    gpio::{p0::Parts as P0Parts, p1::Parts as P1Parts, Level},
    pac::{Peripherals, SPIM0, SPIS1, TIMER2, UARTE0},
    ppi::{Parts as PpiParts, Ppi0},
    spim::{Frequency, Pins as SpimPins, Spim, MODE_0, Error as SpimError},
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

const IL0373_PANEL_SETTING: u8 = 0x00;
const IL0373_POWER_SETTING: u8 = 0x01;
const IL0373_POWER_OFF: u8 = 0x02;
const IL0373_POWER_OFF_SEQUENCE: u8 = 0x03;
const IL0373_POWER_ON: u8 = 0x04;
const IL0373_POWER_ON_MEASURE: u8 = 0x05;
const IL0373_BOOSTER_SOFT_START: u8 = 0x06;
const IL0373_DEEP_SLEEP: u8 = 0x07;
const IL0373_DTM1: u8 = 0x10;
const IL0373_DATA_STOP: u8 = 0x11;
const IL0373_DISPLAY_REFRESH: u8 = 0x12;
const IL0373_DTM2: u8 = 0x13;
const IL0373_PDTM1: u8 = 0x14;
const IL0373_PDTM2: u8 = 0x15;
const IL0373_PDRF: u8 = 0x16;
const IL0373_LUT1: u8 = 0x20;
const IL0373_LUTWW: u8 = 0x21;
const IL0373_LUTBW: u8 = 0x22;
const IL0373_LUTWB: u8 = 0x23;
const IL0373_LUTBB: u8 = 0x24;
const IL0373_PLL: u8 = 0x30;
const IL0373_CDI: u8 = 0x50;
const IL0373_RESOLUTION: u8 = 0x61;
const IL0373_VCM_DC_SETTING: u8 = 0x82;
const IL0373_PARTIAL_WINDOW: u8 = 0x90;
const IL0373_PARTIAL_ENTER: u8 = 0x91;
const IL0373_PARTIAL_EXIT: u8 = 0x92;

const EPD_RAM_BW: u8 = IL0373_DTM1;
const EPD_RAM_RED: u8 = IL0373_DTM2;

// il0373_default_init_code


// void Adafruit_EPD::EPD_commandList(const uint8_t *init_code) {
//   uint8_t buf[64];

//   while (init_code[0] != 0xFE) {
//     uint8_t cmd = init_code[0];
//     init_code++;
//     uint8_t num_args = init_code[0];
//     init_code++;
//     if (cmd == 0xFF) {
//       busy_wait();
//       delay(num_args);
//       continue;
//     }
//     if (num_args > sizeof(buf)) {
//       Serial.println("ERROR - buf not large enough!");
//       while (1)
//         delay(10);
//     }

//     for (int i = 0; i < num_args; i++) {
//       buf[i] = init_code[0];
//       init_code++;
//     }
//     EPD_command(cmd, buf, num_args);
//   }
// }

const IL0373_INIT_CODE: &[u8] = &[
    IL0373_POWER_SETTING,
        5, 0x03, 0x00, 0x2b, 0x2b, 0x09,
    IL0373_BOOSTER_SOFT_START,
        3, 0x17, 0x17, 0x17,
    IL0373_POWER_ON,
        0,

    0xFF, 200, // DELAY SEQUENCE - 200ms

    IL0373_PANEL_SETTING,
        1, 0xCF,
    IL0373_CDI,
        1, 0x37,
    IL0373_PLL,
        1, 0x29,
    IL0373_VCM_DC_SETTING,
        1, 0x0A,

    0xFF, 20, // DELAY SEQUENCE - 20ms

    0xFE // END OF SEQUENCE MARKER
];

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
            wdt.set_lfosc_ticks(32768 * 30);
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
    let gpio1 = P1Parts::new(board.P1);

    // Display parts
    let sclk = gpio0.p0_14;
    let copi = gpio0.p0_13;
    let cipo = gpio0.p0_15;
    let mut tft_dc = gpio0.p0_27.into_push_pull_output(Level::High).degrade();
    let mut tft_cs = gpio0.p0_26.into_push_pull_output(Level::High).degrade();
    let _sram_cs = gpio0.p0_07.into_push_pull_output(Level::High).degrade();
    let _sd_cs = gpio1.p1_08.into_push_pull_output(Level::High).degrade();

    // TODO(AJM): This pin is the "reset pin" by default. Is there anything
    // necessary to "defuse" this before using it?
    // let _epd_reset = gpio.p0_18;

    let spim_pins = SpimPins {
        sck: sclk.into_push_pull_output(Level::Low).degrade(),
        mosi: Some(copi.into_push_pull_output(Level::Low).degrade()),
        miso: Some(cipo.into_floating_input().degrade()),
    };

    let mut spim = Spim::new(
        board.SPIM3,
        spim_pins,
        Frequency::M4,
        MODE_0,
        0
    );

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

    let _bus = BusManagerSimple::new(twim);

    let mut timer = Timer::new(board.TIMER0);

    // HERE BE DRAGONS

    // void begin(thinkinkmode_t mode = THINKINK_TRICOLOR) {
    //     Adafruit_IL0373::begin(true);

            // Adafruit_EPD::begin(reset);

                // {
                //   setBlackBuffer(0, true);  // black defaults to inverted
                //   setColorBuffer(1, false); // red defaults to not inverted

                //   layer_colors[EPD_WHITE] = 0b00;
                //   layer_colors[EPD_BLACK] = 0b01;
                //   layer_colors[EPD_RED] = 0b10;
                //   layer_colors[EPD_GRAY] = 0b10;
                //   layer_colors[EPD_DARK] = 0b01;
                //   layer_colors[EPD_LIGHT] = 0b10;

                //   if (use_sram) {
                //     sram.begin();
                //     sram.write8(0, K640_SEQUENTIAL_MODE, MCPSRAM_WRSR);
                //   }

                //   // Serial.println("set pins");
                //   // set pin directions
                //   pinMode(_dc_pin, OUTPUT);
                //   pinMode(_cs_pin, OUTPUT);

                // #if defined(BUSIO_USE_FAST_PINIO)
                //   csPort = (BusIO_PortReg *)portOutputRegister(digitalPinToPort(_cs_pin));
                //   csPinMask = digitalPinToBitMask(_cs_pin);
                //   dcPort = (BusIO_PortReg *)portOutputRegister(digitalPinToPort(_dc_pin));
                //   dcPinMask = digitalPinToBitMask(_dc_pin);
                // #endif

                //   csHigh();

                //   if (!spi_dev->begin()) {
                //     return;
                //   }

                //   // Serial.println("hard reset");
                //   if (reset) {
                //     hardwareReset();
                //   }

                //   // Serial.println("busy");
                //   if (_busy_pin >= 0) {
                //     pinMode(_busy_pin, INPUT);
                //   }
                //   // Serial.println("done!");
                // }

            // setBlackBuffer(0, true); // black defaults to inverted
            // setColorBuffer(1, true); // red defaults to inverted

            // powerDown();

    //     setColorBuffer(0, true); // layer 0 uninverted
    //     setBlackBuffer(1, true); // layer 1 uninverted

    //     inkmode = mode; // Preserve ink mode for ImageReader or others

    //     layer_colors[EPD_WHITE] = 0b00;
    //     layer_colors[EPD_BLACK] = 0b10;
    //     layer_colors[EPD_RED] = 0b01;
    //     layer_colors[EPD_GRAY] = 0b01;
    //     layer_colors[EPD_LIGHT] = 0b00;
    //     layer_colors[EPD_DARK] = 0b10;

    //     default_refresh_delay = 16000; // AJM

    //     powerDown();
          // // power off
          // uint8_t buf[4];

          // buf[0] = 0x17;
          // EPD_command(IL0373_CDI, buf, 1);

          // buf[0] = 0x00;
          // EPD_command(IL0373_VCM_DC_SETTING, buf, 0);

          // EPD_command(IL0373_POWER_OFF);
    // }

    let mut display = Display {
        spim,
        tft_cs,
        tft_dc,
    };

    // -----
    // This is roughly "power off/down"
    display.tft_cs.set_high().ok();
    display.tft_dc.set_high().ok();
    timer.delay_us(100u32); // AJM guess

    display.command(IL0373_CDI, Some(&[0x17])).unwrap();
    display.command(IL0373_VCM_DC_SETTING, None).unwrap(); // TODO, MAAAYBE send a 0?
    display.command(IL0373_POWER_OFF, None).unwrap();

    //TODO: Guess
    timer.delay_ms(100u32);

    // -----
    // This is roughly "power up"
    defmt::info!("Power Up");

    // IL0373_POWER_SETTING,
    //     5, 0x03, 0x00, 0x2b, 0x2b, 0x09,
    display.command(
        IL0373_POWER_SETTING,
        Some(&[0x03, 0x00, 0x2b, 0x2b, 0x09]),
    ).unwrap();

    // IL0373_BOOSTER_SOFT_START,
    //     3, 0x17, 0x17, 0x17,
    display.command(
        IL0373_BOOSTER_SOFT_START,
        Some(&[0x17, 0x17, 0x17]),
    ).unwrap();

    // IL0373_POWER_ON,
    //     0,
    display.command(IL0373_POWER_ON, None).unwrap();

    // 0xFF, 200, // DELAY SEQUENCE - 200ms
    timer.delay_ms(200u32);

    // IL0373_PANEL_SETTING,
    //     1, 0xCF,
    display.command(IL0373_PANEL_SETTING, Some(&[0xCF])).unwrap();

    // IL0373_CDI,
    //     1, 0x37,
    display.command(IL0373_CDI, Some(&[0x37])).unwrap();


    // IL0373_PLL,
    //     1, 0x29,
    display.command(IL0373_PLL, Some(&[0x29])).unwrap();


    // IL0373_VCM_DC_SETTING,
    //     1, 0x0A,
    display.command(IL0373_VCM_DC_SETTING, Some(&[0x0A])).unwrap();

    // 0xFF, 20, // DELAY SEQUENCE - 20ms
    timer.delay_ms(20u32);

    // SET RESOLUTION
    display.command(
        IL0373_RESOLUTION,
        Some(&[
            104 & 0xFF,
            (212 >> 8) & 0xFF,
            212 & 0xFF,
        ]),
    ).unwrap();

    // TODO: FILL AND SHOW THE DISPLAY
    defmt::info!("Filling display...");

    let mut buf = [0xFFu8; 2756];
    buf[..(13 * 5)].copy_from_slice(&[0x00; (13 * 5)]);

    // writeRAMFramebufferToEPD(buffer1, buffer1_size, 0);
    display.command(EPD_RAM_BW, Some(&buf)).unwrap();

    timer.delay_ms(2u32);
    let mut buf = [0xFFu8; 2756];
    buf[(13 * 5)..(13 * 5 * 2)].copy_from_slice(&[0x00; (13 * 5)]);

    // writeRAMFramebufferToEPD(buffer2, buffer2_size, 1);
    display.command(EPD_RAM_RED, Some(&buf)).unwrap();

    // update();
    defmt::info!("Refresh and wait 20s...");
    display.command(IL0373_DISPLAY_REFRESH, None).unwrap();
    timer.delay_ms(20_000u32);

    // -----
    // This is roughly "power down"
    defmt::info!("Power down...");
    display.command(IL0373_CDI, Some(&[0x17])).unwrap();
    display.command(IL0373_VCM_DC_SETTING, None).unwrap(); // TODO, MAAAYBE send a 0?
    display.command(IL0373_POWER_OFF, None).unwrap();


    // SORRY

    defmt::warn!("WAIT TO REFLASH...");
    fleet_clock::exit()
}

struct Display<S, G>
where
    S: SpimWrite<u8>,
    G: OutputPin,
{
    spim: S,
    tft_dc: G,
    tft_cs: G,
}

impl<S, G> Display<S, G>
where
    S: SpimWrite<u8>,
    G: OutputPin,
{
    fn command(
        &mut self,
        command: u8,
        data: Option<&[u8]>,
    ) -> Result<(), S::Error> {
        self.tft_cs.set_high().ok();
        self.tft_dc.set_low().ok();
        self.tft_cs.set_low().ok();
        self.spim.write(&[command])?;
        self.tft_dc.set_high().ok();
        if let Some(data) = data {
            self.spim.write(data)?;
        }
        self.tft_cs.set_high().ok();
        Ok(())
    }
}

// fn command<T: SpimWrite<U, Error=SpimError>, U>(
//     spim: &mut T,
//     command: u8,
//     data: Option<&[u8]>,
// ) {
//     tft_dc.set_low().ok();
//     spim.write(&[IL0373_CDI]).ok();
//     tft_dc.set_high().ok();
//     spim.write(&[0x17]).ok();
//     tft_cs.set_high().ok();
// }

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
