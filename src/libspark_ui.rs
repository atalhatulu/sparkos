//! SparkOS Desktop V1.20 — Native GUI Framework (`libspark_ui`)
//!
//! Provides a user-space widget architecture (Button, Label, Panel, TextBox),
//! layout engine (VerticalLayout, HorizontalLayout), and surface-isolated event routing.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < (self.x + self.w as i32) && py >= self.y && py < (self.y + self.h as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetEvent {
    MouseClick { x: i32, y: i32 },
    MouseMove { x: i32, y: i32 },
    KeyPress { key_code: u8 },
}

pub trait Widget {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32);
    fn handle_event(&mut self, event: &WidgetEvent) -> bool;
    fn update(&mut self);
    fn bounds(&self) -> Rect;
}

// -----------------------------------------------------------------------------
// 1. Button Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Button {
    pub bounds: Rect,
    pub label: String,
    pub bg_color: u32,
    pub fg_color: u32,
    pub hovered: bool,
    pub clicked: bool,
}

impl Button {
    pub fn new(x: i32, y: i32, w: u32, h: u32, label: &str) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            label: String::from(label),
            bg_color: 0x001E293B, // Slate Dark
            fg_color: 0x00F8FAFC, // Crisp White
            hovered: false,
            clicked: false,
        }
    }
}

impl Widget for Button {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        let bg = if self.clicked {
            0x002563EB // Vibrant Blue
        } else if self.hovered {
            0x00334155 // Lighter Slate
        } else {
            self.bg_color
        };

        // Draw button background
        for row in 0..self.bounds.h {
            let py = self.bounds.y + row as i32;
            if py < 0 || py >= surf_h as i32 { continue; }
            for col in 0..self.bounds.w {
                let px = self.bounds.x + col as i32;
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), bg);
                }
            }
        }

        // Draw button text centered
        let text_x = (self.bounds.x + 8).max(0) as u32;
        let text_y = (self.bounds.y + (self.bounds.h as i32 - 8) / 2).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, text_x, text_y, &self.label, self.fg_color, bg);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        match event {
            WidgetEvent::MouseMove { x, y } => {
                let was_hovered = self.hovered;
                self.hovered = self.bounds.contains(*x, *y);
                self.hovered != was_hovered
            }
            WidgetEvent::MouseClick { x, y } => {
                if self.bounds.contains(*x, *y) {
                    self.clicked = true;
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn update(&mut self) {
        if self.clicked {
            self.clicked = false;
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }
}

// -----------------------------------------------------------------------------
// 2. Label Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Label {
    pub x: i32,
    pub y: i32,
    pub text: String,
    pub fg_color: u32,
    pub bg_color: u32,
}

impl Label {
    pub fn new(x: i32, y: i32, text: &str, fg_color: u32, bg_color: u32) -> Self {
        Self {
            x,
            y,
            text: String::from(text),
            fg_color,
            bg_color,
        }
    }
}

impl Widget for Label {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() || self.x < 0 || self.y < 0 { return; }
        crate::font::draw_text(surface_ptr, surf_w, surf_h, self.x as u32, self.y as u32, &self.text, self.fg_color, self.bg_color);
    }

    fn handle_event(&mut self, _event: &WidgetEvent) -> bool {
        false
    }

    fn update(&mut self) {}

    fn bounds(&self) -> Rect {
        Rect::new(self.x, self.y, (self.text.len() * 8) as u32, 8)
    }
}

// -----------------------------------------------------------------------------
// 3. Panel Widget
// -----------------------------------------------------------------------------
pub struct Panel {
    pub bounds: Rect,
    pub bg_color: u32,
    pub children: Vec<Box<dyn Widget>>,
}

impl Panel {
    pub fn new(x: i32, y: i32, w: u32, h: u32, bg_color: u32) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            bg_color,
            children: Vec::new(),
        }
    }

    pub fn add_child(&mut self, child: Box<dyn Widget>) {
        self.children.push(child);
    }
}

impl Widget for Panel {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        for row in 0..self.bounds.h {
            let py = self.bounds.y + row as i32;
            if py < 0 || py >= surf_h as i32 { continue; }
            for col in 0..self.bounds.w {
                let px = self.bounds.x + col as i32;
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), self.bg_color);
                }
            }
        }
        for child in &self.children {
            child.draw(surface_ptr, surf_w, surf_h);
        }
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        let mut handled = false;
        for child in &mut self.children {
            if child.handle_event(event) {
                handled = true;
            }
        }
        handled
    }

    fn update(&mut self) {
        for child in &mut self.children {
            child.update();
        }
    }

    fn bounds(&self) -> Rect {
        self.bounds
    }
}

// -----------------------------------------------------------------------------
// 4. TextBox Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct TextBox {
    pub bounds: Rect,
    pub text: String,
    pub is_focused: bool,
    pub fg_color: u32,
    pub bg_color: u32,
}

impl TextBox {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            text: String::new(),
            is_focused: false,
            fg_color: 0x00E2E8F0,
            bg_color: 0x000F172A,
        }
    }
}

impl Widget for TextBox {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        let border_color = if self.is_focused { 0x0038BDF8 } else { 0x00334155 };

        for row in 0..self.bounds.h {
            let py = self.bounds.y + row as i32;
            if py < 0 || py >= surf_h as i32 { continue; }
            for col in 0..self.bounds.w {
                let px = self.bounds.x + col as i32;
                if px < 0 || px >= surf_w as i32 { continue; }
                let is_border = row == 0 || row == self.bounds.h - 1 || col == 0 || col == self.bounds.w - 1;
                let col_val = if is_border { border_color } else { self.bg_color };
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), col_val);
                }
            }
        }

        let text_x = (self.bounds.x + 4).max(0) as u32;
        let text_y = (self.bounds.y + (self.bounds.h as i32 - 8) / 2).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, text_x, text_y, &self.text, self.fg_color, self.bg_color);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        match event {
            WidgetEvent::MouseClick { x, y } => {
                self.is_focused = self.bounds.contains(*x, *y);
                self.is_focused
            }
            WidgetEvent::KeyPress { key_code } => {
                if self.is_focused {
                    if *key_code == 8 {
                        self.text.pop();
                        true
                    } else if *key_code >= 32 && *key_code <= 126 {
                        self.text.push(*key_code as char);
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn update(&mut self) {}

    fn bounds(&self) -> Rect {
        self.bounds
    }
}

// -----------------------------------------------------------------------------
// 5. Layout Engine
// -----------------------------------------------------------------------------
pub struct VerticalLayout {
    pub start_x: i32,
    pub start_y: i32,
    pub spacing: u32,
}

impl VerticalLayout {
    pub fn new(start_x: i32, start_y: i32, spacing: u32) -> Self {
        Self { start_x, start_y, spacing }
    }

    pub fn compute_total_height(&self, items: &[&dyn Widget]) -> u32 {
        let mut total = 0u32;
        for item in items {
            total += item.bounds().h + self.spacing;
        }
        total
    }
}

pub struct HorizontalLayout {
    pub start_x: i32,
    pub start_y: i32,
    pub spacing: u32,
}

impl HorizontalLayout {
    pub fn new(start_x: i32, start_y: i32, spacing: u32) -> Self {
        Self { start_x, start_y, spacing }
    }

    pub fn compute_total_width(&self, items: &[&dyn Widget]) -> u32 {
        let mut total = 0u32;
        for item in items {
            total += item.bounds().w + self.spacing;
        }
        total
    }
}
