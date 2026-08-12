use alloc::boxed::Box;

#[derive(Clone, Copy, Debug)]
pub enum UiEvent {
    MouseClick { x: u16, y: u16 },
    MouseUp { x: u16, y: u16 },
    MouseMove { x: u16, y: u16 },
}

pub trait Widget: Send {
    fn draw(&self, x: u16, y: u16);
    fn handle_event(&mut self, event: UiEvent, offset_x: u16, offset_y: u16) -> bool;
    fn bounds(&self) -> (u16, u16); // Width, Height
}
