use alloc::boxed::Box;
use crate::gui::WRITERS;
use crate::ui::{Button, Label};

pub fn init_apps() {
    let mut writers = WRITERS.lock();

    // App 1: Files
    let w1 = &mut writers[1];
    w1.widgets.push(Box::new(Label::new("Root", 10, 40).with_colors(0x00CCCCCC, 0x001A1A1A)));
    w1.widgets.push(Box::new(Button::new("Docs", 10, 60, 100, 30)));
    w1.widgets.push(Box::new(Button::new("Downloads", 10, 100, 100, 30)));
    w1.widgets.push(Box::new(Button::new("Pictures", 10, 140, 100, 30)));

    // App 2: Notepad
    let w2 = &mut writers[2];
    w2.widgets.push(Box::new(Button::new("File", 10, 35, 60, 20)));
    w2.widgets.push(Box::new(Button::new("Edit", 80, 35, 60, 20)));
    w2.widgets.push(Box::new(Label::new("Hello from SparkUI Notepad!", 10, 70).with_colors(0x00E0E0E0, 0x00141414)));
}
