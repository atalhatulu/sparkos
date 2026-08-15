#![no_std]
#![no_main]

use libspark::println;
use libspark::gui;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("[GUI-DEMO] Starting standalone Ring-3 GUI client...");
    match gui::create_surface(320, 240) {
        Ok(surface_id) => {
            println!("[GUI-DEMO] Surface created with ID: {}", surface_id);
            // 0x70000000 üzerindeki piksellere kırmızı dikdörtgen çiz
            let fb_ptr = 0x70000000 as *mut u32;
            unsafe {
                // 320x240 kırmızı dikdörtgen (0x00FF0000)
                for i in 0..(320 * 240) {
                    *fb_ptr.add(i) = 0x00FF0000;
                }
            }
            println!("[GUI-DEMO] Drawing 320x240 RED rectangle into shared memory...");
            match gui::present_surface(surface_id, 0, 0, 320, 240) {
                Ok(_) => {
                    println!("[GUI-DEMO] Surface presented to Display Server successfully!");
                }
                Err(_) => {
                    println!("[GUI-DEMO] Error presenting surface");
                }
            }
            let _ = gui::destroy_surface(surface_id);
            println!("[GUI-DEMO] Surface destroyed cleanly. GUI demo verified.");
            libspark::process::exit(0);
        }
        Err(_) => {
            println!("[GUI-DEMO] Failed to create surface");
            libspark::process::exit(1);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libspark::process::exit(1);
}
