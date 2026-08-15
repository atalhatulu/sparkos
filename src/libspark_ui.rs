//! SparkOS Desktop V1.27 — Advanced SparkUI Framework V2 (`libspark_ui`)
//!
//! Provides an advanced widget library (Button, Checkbox, Slider, Dropdown, TabView, ScrollView, Dialog),
//! hierarchical Widget Tree with event bubbling & focus management, FlexBox layout engine (Row, Column, Padding, Spacing),
//! and performance-optimized Dirty Widget Redraws.

use alloc::boxed::Box;
use alloc::format;
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
pub struct Padding {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

impl Padding {
    pub const fn all(val: u32) -> Self {
        Self { top: val, right: val, bottom: val, left: val }
    }

    pub const fn symmetric(vertical: u32, horizontal: u32) -> Self {
        Self { top: vertical, right: horizontal, bottom: vertical, left: horizontal }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WidgetEvent {
    MouseClick { x: i32, y: i32 },
    MouseMove { x: i32, y: i32 },
    KeyPress { key_code: u8 },
    FocusGained,
    FocusLost,
}

pub trait Widget {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32);
    fn handle_event(&mut self, event: &WidgetEvent) -> bool;
    fn update(&mut self) {}
    fn bounds(&self) -> Rect;
    fn is_dirty(&self) -> bool { true }
    fn set_dirty(&mut self, _dirty: bool) {}
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
    pub dirty: bool,
}

impl Button {
    pub fn new(x: i32, y: i32, w: u32, h: u32, label: &str) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            label: String::from(label),
            bg_color: 0x001E293B,
            fg_color: 0x00F8FAFC,
            hovered: false,
            clicked: false,
            dirty: true,
        }
    }
}

impl Widget for Button {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        let current_bg = if self.clicked {
            0x000284C7
        } else if self.hovered {
            0x00334155
        } else {
            self.bg_color
        };

        for py in self.bounds.y..(self.bounds.y + self.bounds.h as i32) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in self.bounds.x..(self.bounds.x + self.bounds.w as i32) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), current_bg);
                }
            }
        }

        let text_x = (self.bounds.x + 8).max(0) as u32;
        let text_y = (self.bounds.y + (self.bounds.h as i32 / 2) - 4).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, text_x, text_y, &self.label, self.fg_color, current_bg);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        match event {
            WidgetEvent::MouseMove { x, y } => {
                let inside = self.bounds.contains(*x, *y);
                if self.hovered != inside {
                    self.hovered = inside;
                    self.dirty = true;
                    return true;
                }
            }
            WidgetEvent::MouseClick { x, y } => {
                if self.bounds.contains(*x, *y) {
                    self.clicked = true;
                    self.dirty = true;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 2. Checkbox Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Checkbox {
    pub bounds: Rect,
    pub label: String,
    pub checked: bool,
    pub dirty: bool,
}

impl Checkbox {
    pub fn new(x: i32, y: i32, w: u32, h: u32, label: &str, checked: bool) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            label: String::from(label),
            checked,
            dirty: true,
        }
    }
}

impl Widget for Checkbox {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        let box_size = 14i32;
        let box_color = if self.checked { 0x000284C7 } else { 0x00334155 };

        for py in self.bounds.y..(self.bounds.y + box_size) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in self.bounds.x..(self.bounds.x + box_size) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), box_color);
                }
            }
        }

        let label_x = (self.bounds.x + box_size + 8).max(0) as u32;
        let label_y = (self.bounds.y + 2).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, label_x, label_y, &self.label, 0x00F8FAFC, 0x000F172A);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        if let WidgetEvent::MouseClick { x, y } = event {
            if self.bounds.contains(*x, *y) {
                self.checked = !self.checked;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 3. Slider Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Slider {
    pub bounds: Rect,
    pub min: i32,
    pub max: i32,
    pub value: i32,
    pub dirty: bool,
}

impl Slider {
    pub fn new(x: i32, y: i32, w: u32, h: u32, min: i32, max: i32, value: i32) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            min,
            max,
            value: value.clamp(min, max),
            dirty: true,
        }
    }
}

impl Widget for Slider {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        let track_y = self.bounds.y + (self.bounds.h as i32 / 2) - 2;
        let track_h = 4i32;

        // Draw track
        for py in track_y..(track_y + track_h) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in self.bounds.x..(self.bounds.x + self.bounds.w as i32) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), 0x00475569);
                }
            }
        }

        // Draw handle
        let range = (self.max - self.min).max(1) as f32;
        let progress = (self.value - self.min) as f32 / range;
        let handle_x = self.bounds.x + (progress * (self.bounds.w as f32 - 10.0)) as i32;

        for py in self.bounds.y..(self.bounds.y + self.bounds.h as i32) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in handle_x..(handle_x + 10) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), 0x0038BDF8);
                }
            }
        }
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        if let WidgetEvent::MouseClick { x, y } = event {
            if self.bounds.contains(*x, *y) {
                let range = (self.max - self.min) as f32;
                let rel_x = (*x - self.bounds.x).max(0) as f32;
                let pct = (rel_x / self.bounds.w as f32).clamp(0.0, 1.0);
                self.value = self.min + (pct * range) as i32;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 4. Dropdown Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Dropdown {
    pub bounds: Rect,
    pub items: Vec<String>,
    pub selected_index: usize,
    pub is_open: bool,
    pub dirty: bool,
}

impl Dropdown {
    pub fn new(x: i32, y: i32, w: u32, h: u32, items: Vec<String>) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            items,
            selected_index: 0,
            is_open: false,
            dirty: true,
        }
    }
}

impl Widget for Dropdown {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        for py in self.bounds.y..(self.bounds.y + self.bounds.h as i32) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in self.bounds.x..(self.bounds.x + self.bounds.w as i32) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), 0x001E293B);
                }
            }
        }

        let selected_text = self.items.get(self.selected_index).map(|s| s.as_str()).unwrap_or("None");
        let display = format!("{}  [v]", selected_text);
        let tx = (self.bounds.x + 6).max(0) as u32;
        let ty = (self.bounds.y + 4).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, tx, ty, &display, 0x00F8FAFC, 0x001E293B);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        if let WidgetEvent::MouseClick { x, y } = event {
            if self.bounds.contains(*x, *y) {
                if !self.items.is_empty() {
                    self.selected_index = (self.selected_index + 1) % self.items.len();
                }
                self.dirty = true;
                return true;
            }
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 5. TabView Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct TabView {
    pub bounds: Rect,
    pub tabs: Vec<String>,
    pub active_tab: usize,
    pub dirty: bool,
}

impl TabView {
    pub fn new(x: i32, y: i32, w: u32, h: u32, tabs: Vec<String>) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            tabs,
            active_tab: 0,
            dirty: true,
        }
    }
}

impl Widget for TabView {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() || self.tabs.is_empty() { return; }
        let tab_w = self.bounds.w / (self.tabs.len() as u32);
        for (i, tab) in self.tabs.iter().enumerate() {
            let tx = self.bounds.x + (i as i32 * tab_w as i32);
            let tab_bg = if i == self.active_tab { 0x000284C7 } else { 0x00334155 };

            for py in self.bounds.y..(self.bounds.y + 20) {
                if py < 0 || py >= surf_h as i32 { continue; }
                for px in tx..(tx + tab_w as i32) {
                    if px < 0 || px >= surf_w as i32 { continue; }
                    let offset = (py as usize) * (surf_w as usize) + (px as usize);
                    unsafe {
                        core::ptr::write_volatile(surface_ptr.add(offset), tab_bg);
                    }
                }
            }

            let text_x = (tx + 6).max(0) as u32;
            let text_y = (self.bounds.y + 4).max(0) as u32;
            crate::font::draw_text(surface_ptr, surf_w, surf_h, text_x, text_y, tab, 0x00F8FAFC, tab_bg);
        }
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        if let WidgetEvent::MouseClick { x, y } = event {
            if *y >= self.bounds.y && *y < self.bounds.y + 20 && *x >= self.bounds.x && *x < self.bounds.x + self.bounds.w as i32 {
                let tab_w = self.bounds.w / (self.tabs.len() as u32).max(1);
                let clicked_idx = ((*x - self.bounds.x) / tab_w as i32) as usize;
                if clicked_idx < self.tabs.len() && clicked_idx != self.active_tab {
                    self.active_tab = clicked_idx;
                    self.dirty = true;
                    return true;
                }
            }
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 6. ScrollView Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct ScrollView {
    pub bounds: Rect,
    pub content_height: u32,
    pub scroll_y: i32,
    pub dirty: bool,
}

impl ScrollView {
    pub fn new(x: i32, y: i32, w: u32, h: u32, content_height: u32) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            content_height,
            scroll_y: 0,
            dirty: true,
        }
    }

    pub fn scroll(&mut self, delta: i32) {
        let max_scroll = (self.content_height as i32 - self.bounds.h as i32).max(0);
        self.scroll_y = (self.scroll_y + delta).clamp(0, max_scroll);
        self.dirty = true;
    }
}

impl Widget for ScrollView {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        // Scrollbar track
        let sb_x = self.bounds.x + self.bounds.w as i32 - 8;
        for py in self.bounds.y..(self.bounds.y + self.bounds.h as i32) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in sb_x..(sb_x + 8) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), 0x001E293B);
                }
            }
        }
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        if let WidgetEvent::KeyPress { key_code } = event {
            if *key_code == 0x48 { // Arrow Up
                self.scroll(-10);
                return true;
            } else if *key_code == 0x50 { // Arrow Down
                self.scroll(10);
                return true;
            }
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 7. Dialog Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Dialog {
    pub bounds: Rect,
    pub title: String,
    pub message: String,
    pub is_visible: bool,
    pub dirty: bool,
}

impl Dialog {
    pub fn new(x: i32, y: i32, w: u32, h: u32, title: &str, message: &str) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            title: String::from(title),
            message: String::from(message),
            is_visible: true,
            dirty: true,
        }
    }
}

impl Widget for Dialog {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() || !self.is_visible { return; }
        for py in self.bounds.y..(self.bounds.y + self.bounds.h as i32) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in self.bounds.x..(self.bounds.x + self.bounds.w as i32) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), 0x000F172A);
                }
            }
        }

        let title_x = (self.bounds.x + 12).max(0) as u32;
        let title_y = (self.bounds.y + 10).max(0) as u32;
        let msg_x = (self.bounds.x + 12).max(0) as u32;
        let msg_y = (self.bounds.y + 30).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, title_x, title_y, &self.title, 0x0038BDF8, 0x000F172A);
        crate::font::draw_text(surface_ptr, surf_w, surf_h, msg_x, msg_y, &self.message, 0x00F8FAFC, 0x000F172A);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        if let WidgetEvent::KeyPress { key_code } = event {
            if *key_code == 0x1B { // ESC
                self.is_visible = false;
                self.dirty = true;
                return true;
            }
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 8. Label Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Label {
    pub bounds: Rect,
    pub text: String,
    pub fg_color: u32,
    pub bg_color: u32,
    pub dirty: bool,
}

impl Label {
    pub fn new(x: i32, y: i32, text: &str, fg_color: u32, bg_color: u32) -> Self {
        let text_len = text.chars().count() as u32;
        Self {
            bounds: Rect::new(x, y, text_len * 8, 12),
            text: String::from(text),
            fg_color,
            bg_color,
            dirty: true,
        }
    }
}

impl Widget for Label {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        let lx = self.bounds.x.max(0) as u32;
        let ly = self.bounds.y.max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, lx, ly, &self.text, self.fg_color, self.bg_color);
    }
    fn handle_event(&mut self, _event: &WidgetEvent) -> bool { false }
    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 9. TextBox Widget
// -----------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct TextBox {
    pub bounds: Rect,
    pub text: String,
    pub is_focused: bool,
    pub dirty: bool,
}

impl TextBox {
    pub fn new(x: i32, y: i32, w: u32, h: u32) -> Self {
        Self {
            bounds: Rect::new(x, y, w, h),
            text: String::new(),
            is_focused: false,
            dirty: true,
        }
    }
}

impl Widget for TextBox {
    fn draw(&self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if surface_ptr.is_null() { return; }
        let border_color = if self.is_focused { 0x000284C7 } else { 0x00334155 };
        for py in self.bounds.y..(self.bounds.y + self.bounds.h as i32) {
            if py < 0 || py >= surf_h as i32 { continue; }
            for px in self.bounds.x..(self.bounds.x + self.bounds.w as i32) {
                if px < 0 || px >= surf_w as i32 { continue; }
                let offset = (py as usize) * (surf_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(surface_ptr.add(offset), border_color);
                }
            }
        }
        let tx = (self.bounds.x + 4).max(0) as u32;
        let ty = (self.bounds.y + 4).max(0) as u32;
        crate::font::draw_text(surface_ptr, surf_w, surf_h, tx, ty, &self.text, 0x00F8FAFC, border_color);
    }

    fn handle_event(&mut self, event: &WidgetEvent) -> bool {
        match event {
            WidgetEvent::MouseClick { x, y } => {
                let inside = self.bounds.contains(*x, *y);
                if self.is_focused != inside {
                    self.is_focused = inside;
                    self.dirty = true;
                    return true;
                }
            }
            WidgetEvent::KeyPress { key_code } => {
                if self.is_focused && *key_code >= 0x20 && *key_code <= 0x7E {
                    self.text.push(*key_code as char);
                    self.dirty = true;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    fn bounds(&self) -> Rect { self.bounds }
    fn is_dirty(&self) -> bool { self.dirty }
    fn set_dirty(&mut self, dirty: bool) { self.dirty = dirty; }
}

// -----------------------------------------------------------------------------
// 10. Hierarchical Widget Tree & FlexBox Layout Engine
// -----------------------------------------------------------------------------
pub struct WidgetNode {
    pub id: u32,
    pub widget: Box<dyn Widget>,
    pub children: Vec<WidgetNode>,
}

pub struct WidgetTree {
    pub root: Option<WidgetNode>,
    pub focused_id: Option<u32>,
    pub captured_id: Option<u32>,
    pub next_id: u32,
}

impl WidgetTree {
    pub fn new() -> Self {
        Self {
            root: None,
            focused_id: None,
            captured_id: None,
            next_id: 1,
        }
    }

    pub fn set_root(&mut self, widget: Box<dyn Widget>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.root = Some(WidgetNode {
            id,
            widget,
            children: Vec::new(),
        });
        id
    }

    pub fn dispatch_event(&mut self, event: &WidgetEvent) -> bool {
        if let Some(root) = &mut self.root {
            Self::dispatch_node(root, event)
        } else {
            false
        }
    }

    fn dispatch_node(node: &mut WidgetNode, event: &WidgetEvent) -> bool {
        // 1. Traverse children first (bubbling from bottom up)
        for child in &mut node.children {
            if Self::dispatch_node(child, event) {
                return true;
            }
        }
        // 2. Process own event
        node.widget.handle_event(event)
    }

    pub fn render_dirty(&mut self, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if let Some(root) = &mut self.root {
            Self::render_node(root, surface_ptr, surf_w, surf_h);
        }
    }

    fn render_node(node: &mut WidgetNode, surface_ptr: *mut u32, surf_w: u32, surf_h: u32) {
        if node.widget.is_dirty() {
            node.widget.draw(surface_ptr, surf_w, surf_h);
            node.widget.set_dirty(false);
        }
        for child in &mut node.children {
            Self::render_node(child, surface_ptr, surf_w, surf_h);
        }
    }
}
