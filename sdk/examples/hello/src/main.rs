#![no_std]
#![no_main]

use libspark::println;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("Hello from SparkOS Official Developer SDK!");
    libspark::process::exit(0);
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libspark::process::exit(1);
}
