//! SparkOS Desktop V1.2 — Mouse Cursor System
//!
//! Provides cursor state tracking, pluggable 16x16 monochrome cursor bitmaps,
//! bounds clamping, and top-layer compositor rendering without modifying client surface memory.

use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorType {
    Default,
    Text,
    ResizeHorizontal,
    ResizeVertical,
    ResizeDiagonal,
    Hand,
}

#[derive(Debug, Clone, Copy)]
pub struct CursorState {
    pub x: u16,
    pub y: u16,
    pub visible: bool,
    pub pressed_button: bool,
    pub cursor_type: CursorType,
}

impl CursorState {
    pub const fn new() -> Self {
        Self {
            x: 320,
            y: 180,
            visible: true,
            pressed_button: false,
            cursor_type: CursorType::Default,
        }
    }
}

pub static CURSOR: Mutex<CursorState> = Mutex::new(CursorState::new());

// ---------------------------------------------------------------------------
// 16x16 Monochrome Cursor Bitmaps
// '*' = Black outline (0x00000000), '.' = White fill (0x00FFFFFF), ' ' = Transparent
// ---------------------------------------------------------------------------

const CURSOR_DEFAULT: [&[u8; 16]; 16] = [
    b"*               ",
    b"**              ",
    b"*.*             ",
    b"*..*            ",
    b"*...*           ",
    b"*....*          ",
    b"*.....*         ",
    b"*......*        ",
    b"*.......*       ",
    b"*........*      ",
    b"*.........*     ",
    b"*.....*****     ",
    b"*..*..*         ",
    b"*.* *..*        ",
    b"**   *..*       ",
    b"*     **        ",
];

const CURSOR_TEXT: [&[u8; 16]; 16] = [
    b"******   ****** ",
    b"  *         *   ",
    b"  *    *    *   ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"       *        ",
    b"  *    *    *   ",
    b"  *         *   ",
    b"******   ****** ",
    b"                ",
];

const CURSOR_RESIZE_H: [&[u8; 16]; 16] = [
    b"                ",
    b"                ",
    b"                ",
    b"                ",
    b"   *        *   ",
    b"  **        **  ",
    b" *.*        *.* ",
    b"****************",
    b"****************",
    b" *.*        *.* ",
    b"  **        **  ",
    b"   *        *   ",
    b"                ",
    b"                ",
    b"                ",
    b"                ",
];

const CURSOR_RESIZE_V: [&[u8; 16]; 16] = [
    b"       **       ",
    b"      ****      ",
    b"     **..**     ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"       **       ",
    b"     **..**     ",
    b"      ****      ",
    b"       **       ",
    b"                ",
];

const CURSOR_RESIZE_DIAG: [&[u8; 16]; 16] = [
    b" ******         ",
    b" *....*         ",
    b" *..*           ",
    b" *.* *          ",
    b" **   *         ",
    b" *     *        ",
    b"        *       ",
    b"         *      ",
    b"        * *     ",
    b"       *   **   ",
    b"      *     *   ",
    b"     *    * *.* ",
    b"         *..*.* ",
    b"        *....*  ",
    b"        ******  ",
    b"                ",
];

const CURSOR_HAND: [&[u8; 16]; 16] = [
    b"   ***          ",
    b"  *..*          ",
    b"  *..*          ",
    b"  *..*          ",
    b"  *..*   ***    ",
    b"  *..*  *..*    ",
    b"  *..* *..*..*  ",
    b"  *..**..*..*.* ",
    b" ***.*..*..*..* ",
    b"*..*.*..*..*..* ",
    b"*..*..........* ",
    b" *............* ",
    b"  *..........*  ",
    b"   *........*   ",
    b"    *......*    ",
    b"     ******     ",
];

pub fn get_cursor_bitmap(cursor_type: CursorType) -> &'static [&'static [u8; 16]; 16] {
    match cursor_type {
        CursorType::Default => &CURSOR_DEFAULT,
        CursorType::Text => &CURSOR_TEXT,
        CursorType::ResizeHorizontal => &CURSOR_RESIZE_H,
        CursorType::ResizeVertical => &CURSOR_RESIZE_V,
        CursorType::ResizeDiagonal => &CURSOR_RESIZE_DIAG,
        CursorType::Hand => &CURSOR_HAND,
    }
}

/// Updates mouse cursor coordinates and click state with strict bounds enforcement.
pub fn update_mouse_input(x: i16, y: i16, left_click: bool, screen_w: u16, screen_h: u16) {
    let mut state = CURSOR.lock();
    let cx = (x.max(0) as u16).min(screen_w.saturating_sub(1));
    let cy = (y.max(0) as u16).min(screen_h.saturating_sub(1));
    state.x = cx;
    state.y = cy;
    state.pressed_button = left_click;
}

pub fn set_cursor_type(cursor_type: CursorType) {
    let mut state = CURSOR.lock();
    state.cursor_type = cursor_type;
}

pub fn set_cursor_visible(visible: bool) {
    let mut state = CURSOR.lock();
    state.visible = visible;
}

pub fn get_cursor_state() -> CursorState {
    *CURSOR.lock()
}

/// Top-layer compositor rendering: draws the cursor on the Backbuffer without modifying surface memory.
pub fn draw_cursor_layer() {
    let state = *CURSOR.lock();
    if !state.visible {
        return;
    }

    let screen_w = unsafe { crate::gui::VESA.width };
    let screen_h = unsafe { crate::gui::VESA.height };
    let backbuffer = unsafe { crate::gui::BACKBUFFER };

    if backbuffer.is_null() {
        return;
    }

    let bitmap = get_cursor_bitmap(state.cursor_type);

    for row in 0..16u16 {
        let py = state.y + row;
        if py >= screen_h {
            break;
        }

        for col in 0..16u16 {
            let px = state.x + col;
            if px >= screen_w {
                break;
            }

            let symbol = bitmap[row as usize][col as usize];
            if symbol == b'*' {
                let offset = (py as usize) * (screen_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(backbuffer.add(offset), 0x00000000); // Black Outline
                }
            } else if symbol == b'.' {
                let offset = (py as usize) * (screen_w as usize) + (px as usize);
                unsafe {
                    core::ptr::write_volatile(backbuffer.add(offset), 0x00FFFFFF); // White Fill
                }
            }
        }
    }
}
