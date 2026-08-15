#![no_std]
#![no_main]

use libspark::println;
use libspark::gui::{self, Rect, Canvas};
use libspark::terminal::TextGrid;

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("[TERMINAL] Starting SparkOS Graphical Terminal...");

    let width = 320u32;
    let height = 240u32;

    match gui::create_surface(width, height) {
        Ok(surface_id) => {
            println!("[TERMINAL] Surface allocated (ID: {})", surface_id);

            let fb_ptr = 0x70000000 as *mut u32;
            let fb_slice = unsafe { core::slice::from_raw_parts_mut(fb_ptr, (width * height) as usize) };
            let mut canvas = Canvas::new(fb_slice, width, height);

            // Dark Terminal Background
            canvas.fill_rect(&Rect::new(0, 0, width, height), 0xFF0F172A);
            canvas.draw_border(&Rect::new(0, 0, width, height), 2, 0xFF38BDF8);

            // Initialize 40x15 Text Grid
            let mut grid = TextGrid::<40, 15>::new(0xFF22C55E, 0xFF0F172A);

            // Simulate shell commands and output
            grid.write_str("sparkos$ echo hello\n", 0xFFE2E8F0, 0xFF0F172A);
            grid.write_str("hello\n", 0xFF22C55E, 0xFF0F172A);
            grid.write_str("sparkos$ /bin/hello\n", 0xFFE2E8F0, 0xFF0F172A);
            grid.write_str("Hello, SparkOS World from Ring 3!\n", 0xFF38BDF8, 0xFF0F172A);
            grid.write_str("sparkos$ ", 0xFFE2E8F0, 0xFF0F172A);

            // Draw block cursor
            let cursor_pixel_x = (grid.cursor_x * 8) as i32 + 4;
            let cursor_pixel_y = (grid.cursor_y * 16) as i32 + 4;
            canvas.fill_rect(&Rect::new(cursor_pixel_x, cursor_pixel_y, 8, 14), 0xFF22C55E);

            // Present to Display Server
            let _ = gui::present_surface(surface_id, 0, 0, width, height);
            println!("[TERMINAL] Terminal frame rendered & presented successfully.");

            let _ = gui::destroy_surface(surface_id);
            println!("[TERMINAL] Graphical Terminal session finished cleanly.");
            libspark::process::exit(0);
        }
        Err(_) => {
            println!("[TERMINAL] Failed to create surface");
            libspark::process::exit(1);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libspark::process::exit(1);
}
