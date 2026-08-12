use alloc::string::String;
use crate::gui::{draw_rect, draw_char};
use super::widget::{Widget, UiEvent};

pub struct Button {
    pub text: String,
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    pub bg_color: u32,
    pub fg_color: u32,
    pub pressed: bool,
    pub on_click: Option<fn()>,
}

impl Button {
    pub fn new(text: &str, x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            text: String::from(text),
            x,
            y,
            width,
            height,
            bg_color: 0x003A3A3A,
            fg_color: 0x00E0E0E0,
            pressed: false,
            on_click: None,
        }
    }

    pub fn with_click(mut self, handler: fn()) -> Self {
        self.on_click = Some(handler);
        self
    }
}

impl Widget for Button {
    fn draw(&self, offset_x: u16, offset_y: u16) {
        let abs_x = offset_x + self.x;
        let abs_y = offset_y + self.y;
        
        let color = if self.pressed { 0x005A5A5A } else { self.bg_color };
        draw_rect(abs_x, abs_y, self.width, self.height, color);
        
        let text_width = (self.text.len() * 8) as u16;
        let px = abs_x + (self.width.saturating_sub(text_width)) / 2;
        let py = abs_y + (self.height.saturating_sub(8)) / 2;
        
        let mut cur_x = px;
        for c in self.text.chars() {
            draw_char(cur_x, py, c, self.fg_color, color);
            cur_x += 8;
        }
    }

    fn handle_event(&mut self, event: UiEvent, offset_x: u16, offset_y: u16) -> bool {
        let abs_x = offset_x + self.x;
        let abs_y = offset_y + self.y;
        match event {
            UiEvent::MouseClick { x, y } => {
                if x >= abs_x && x <= abs_x + self.width && y >= abs_y && y <= abs_y + self.height {
                    self.pressed = true;
                    crate::gui::redraw_all(Some((abs_x, abs_y, self.width, self.height)));
                    return true;
                }
            },
            UiEvent::MouseUp { x, y } => {
                if self.pressed {
                    self.pressed = false;
                    crate::gui::redraw_all(Some((abs_x, abs_y, self.width, self.height)));
                    
                    if x >= abs_x && x <= abs_x + self.width && y >= abs_y && y <= abs_y + self.height {
                        if let Some(handler) = self.on_click {
                            handler();
                        }
                        return true;
                    }
                }
            },
            _ => {}
        }
        false
    }

    fn bounds(&self) -> (u16, u16) {
        (self.width, self.height)
    }
}
