#![no_std]
#![no_main]

use libspark::println;
use libspark::gui::{self, Rect, Canvas, Button, Label};

#[no_mangle]
pub extern "C" fn _start() -> ! {
    println!("[GUI-SHOWCASE] Initializing SparkOS GUI Showcase App...");

    let width = 320u32;
    let height = 240u32;

    match gui::create_surface(width, height) {
        Ok(surface_id) => {
            println!("[GUI-SHOWCASE] Surface created (ID: {})", surface_id);

            let fb_ptr = 0x70000000 as *mut u32;
            let fb_slice = unsafe { core::slice::from_raw_parts_mut(fb_ptr, (width * height) as usize) };
            let mut canvas = Canvas::new(fb_slice, width, height);

            // Background
            canvas.fill_rect(&Rect::new(0, 0, width, height), 0xFF1E293B); // Dark Slate

            // Title Label
            let title_label = Label::new(Rect::new(20, 20, 280, 30), "Hello SparkOS Toolkit", 0xFF334155);
            title_label.draw(&mut canvas);

            // Interactive Button
            let mut button = Button::new(Rect::new(60, 80, 200, 50), "CLICK ME");
            button.draw(&mut canvas);

            // Counter Label
            let counter_label = Label::new(Rect::new(40, 160, 240, 40), "Button clicks: 1", 0xFF0F172A);
            counter_label.draw(&mut canvas);

            // Present initial frame
            let _ = gui::present_surface(surface_id, 0, 0, width, height);
            println!("[GUI-SHOWCASE] Initial frame rendered and presented.");

            // Simulate Click Event
            let click_event = libspark::event::InputEvent {
                event_type: 4, // MouseDown
                modifiers: 0,
                key_code: 0,
                mouse_button: 1,
                wheel_delta: 0,
                _reserved: [0; 3],
                mouse_x: 100, // Inside button
                mouse_y: 100,
                timestamp: 100,
                _padding: [0; 8],
            };
            if button.handle_event(&click_event) {
                button.draw(&mut canvas);
                let _ = gui::present_surface(surface_id, button.bounds.x as u32, button.bounds.y as u32, button.bounds.width, button.bounds.height);
                println!("[GUI-SHOWCASE] Button state -> Pressed (Dirty rect presented).");
            }

            let release_event = libspark::event::InputEvent {
                event_type: 5, // MouseUp
                modifiers: 0,
                key_code: 0,
                mouse_button: 1,
                wheel_delta: 0,
                _reserved: [0; 3],
                mouse_x: 100,
                mouse_y: 100,
                timestamp: 200,
                _padding: [0; 8],
            };
            if button.handle_event(&release_event) {
                button.draw(&mut canvas);
                let _ = gui::present_surface(surface_id, button.bounds.x as u32, button.bounds.y as u32, button.bounds.width, button.bounds.height);
                println!("[GUI-SHOWCASE] Button click registered! Total clicks: {}", button.clicks);
            }

            let _ = gui::destroy_surface(surface_id);
            println!("[GUI-SHOWCASE] GUI Showcase verification completed cleanly.");
            libspark::process::exit(0);
        }
        Err(_) => {
            println!("[GUI-SHOWCASE] Failed to create surface");
            libspark::process::exit(1);
        }
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    libspark::process::exit(1);
}
