#![no_std]

use core::sync::atomic::{AtomicUsize, Ordering};

use defmt_rtt as _; // global logger
use nrf52840_hal as _; // memory layout

use panic_probe as _;

// same panicking *behavior* as `panic-probe` but doesn't print a panic message
// this prevents the panic message being printed *twice* when `defmt::panic` is invoked
#[cfg(not(feature = "panic-reset"))]
#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::asm::udf()
}

#[cfg(feature = "panic-reset")]
#[defmt::panic_handler]
fn panic() -> ! {
    cortex_m::peripheral::SCB::sys_reset()
}

// #[defmt::timestamp]
// fn timestamp() -> u64 {
//     static COUNT: AtomicUsize = AtomicUsize::new(0);
//     // NOTE(no-CAS) `timestamps` runs with interrupts disabled
//     let n = COUNT.load(Ordering::Relaxed);
//     COUNT.store(n + 1, Ordering::Relaxed);
//     n as u64
// }

/// Terminates the application and makes `probe-run` exit with exit-code = 0
pub fn exit() -> ! {
    loop {
        cortex_m::asm::bkpt();
    }
}
