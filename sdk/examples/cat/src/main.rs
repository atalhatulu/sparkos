#![no_std]
#![no_main]

use libspark::println;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    match libspark::fd::open("/etc/resolv.conf", libspark::fd::O_RDONLY) {
        Ok(fd) => {
            let mut buf = [0u8; 128];
            if let Ok(len) = libspark::fd::read(fd, &mut buf) {
                if let Ok(s) = core::str::from_utf8(&buf[..len]) {
                    println!("{}", s);
                }
            }
            let _ = libspark::fd::close(fd);
            libspark::process::exit(0);
        }
        Err(_) => {
            println!("cat: failed to open file");
            libspark::process::exit(1);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libspark::process::exit(1);
}
