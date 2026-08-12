use alloc::string::String;
use crate::gui::{draw_rect, draw_char};
use super::widget::{Widget, UiEvent};

pub struct Label {
    pub text: String,
    pub x: u16,
    pub y: u16,
    pub fg_color: u32,
    pub bg_color: u32, // If alpha blending is supported, 0x00000000 means transparent
}

impl Label {
    pub fn new(text: &str, x: u16, y: u16) -> Self {
        Self {
            text: String::from(text),
            x,
            y,
            fg_color: 0x00E0E0E0,
            bg_color: 0x00141414,
        }
    }

    pub fn with_colors(mut self, fg: u32, bg: u32) -> Self {
        self.fg_color = fg;
        self.bg_color = bg;
        self
    }
}

impl Widget for Label {
    fn draw(&self, offset_x: u16, offset_y: u16) {
        let text_width = (self.text.len() * 8) as u16;
        let abs_x = offset_x + self.x;
        let abs_y = offset_y + self.y;
        
        // Draw background if not completely transparent
        if self.bg_color != 0x00000000 {
            draw_rect(abs_x, abs_y, text_width, 8, self.bg_color);
        }

        let mut cur_x = abs_x;
        for c in self.text.chars() {
            draw_char(cur_x, abs_y, c, self.fg_color, self.bg_color);
            cur_x += 8;
        }
    }

    fn handle_event(&mut self, _event: UiEvent, _offset_x: u16, _offset_y: u16) -> bool {
        false // Labels usually don't handle events
    }

    fn bounds(&self) -> (u16, u16) {
        ((self.text.len() * 8) as u16, 8)
    }
}
