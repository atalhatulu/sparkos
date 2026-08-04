use alloc::string::{String, ToString};
use alloc::vec::Vec;
use crate::vga_buffer::{Color, WRITE_LOCK};
use crate::keyboard::Key;
use core::fmt::Write;

const COLS: usize = 80;
const ROWS: usize = 25;

pub async fn run_editor(path: &str, display_name: &str) {
    let mut content = crate::fs::read_file(path).unwrap_or_else(|_| String::new());
    
    // Split into lines
    let mut lines: Vec<String> = content.split('\n').map(|s| s.to_string()).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }

    let mut cursor_x = 0;
    let mut cursor_y = 0;
    let mut scroll_y = 0;

    loop {
        draw_screen(&lines, cursor_x, cursor_y, scroll_y, display_name);

        let mut key = None;
        while key.is_none() {
            while let Some(k) = crate::keyboard::read_key() {
                key = Some(k);
                break;
            }
            if key.is_none() {
                let scancode = crate::task::keyboard::read_scancode().await;
                crate::keyboard::KEYBOARD.lock().handle_scancode(scancode);
            }
        }

        match key.unwrap() {
            Key::Escape => {
                // Save and exit
                let mut saved = String::new();
                for (i, line) in lines.iter().enumerate() {
                    saved.push_str(line);
                    if i < lines.len() - 1 {
                        saved.push('\n');
                    }
                }
                let _ = crate::fs::write_file(path, &saved);
                
                let mut w = WRITE_LOCK.lock();
                w.clear();
                w.set_color(Color::Green, Color::Black);
                writeln!(w, "Dosya kaydedildi: {}", display_name).unwrap();
                return;
            }
            Key::Up => {
                if cursor_y > 0 {
                    cursor_y -= 1;
                    if cursor_x > lines[cursor_y].len() {
                        cursor_x = lines[cursor_y].len();
                    }
                    if cursor_y < scroll_y {
                        scroll_y = cursor_y;
                    }
                }
            }
            Key::Down => {
                if cursor_y < lines.len() - 1 {
                    cursor_y += 1;
                    if cursor_x > lines[cursor_y].len() {
                        cursor_x = lines[cursor_y].len();
                    }
                    if cursor_y >= scroll_y + ROWS - 2 {
                        scroll_y = cursor_y - (ROWS - 3);
                    }
                }
            }
            Key::Left => {
                if cursor_x > 0 {
                    cursor_x -= 1;
                } else if cursor_y > 0 {
                    cursor_y -= 1;
                    cursor_x = lines[cursor_y].len();
                    if cursor_y < scroll_y { scroll_y = cursor_y; }
                }
            }
            Key::Right => {
                if cursor_x < lines[cursor_y].len() {
                    cursor_x += 1;
                } else if cursor_y < lines.len() - 1 {
                    cursor_y += 1;
                    cursor_x = 0;
                    if cursor_y >= scroll_y + ROWS - 2 { scroll_y = cursor_y - (ROWS - 3); }
                }
            }
            Key::Enter => {
                let right_part = lines[cursor_y][cursor_x..].to_string();
                lines[cursor_y].truncate(cursor_x);
                lines.insert(cursor_y + 1, right_part);
                cursor_y += 1;
                cursor_x = 0;
                if cursor_y >= scroll_y + ROWS - 2 { scroll_y = cursor_y - (ROWS - 3); }
            }
            Key::Backspace => {
                if cursor_x > 0 {
                    lines[cursor_y].remove(cursor_x - 1);
                    cursor_x -= 1;
                } else if cursor_y > 0 {
                    let current_line = lines.remove(cursor_y);
                    cursor_y -= 1;
                    cursor_x = lines[cursor_y].len();
                    lines[cursor_y].push_str(&current_line);
                    if cursor_y < scroll_y { scroll_y = cursor_y; }
                }
            }
            Key::Ascii(c) => {
                lines[cursor_y].insert(cursor_x, c as char);
                cursor_x += 1;
            }
            _ => {}
        }
    }
}

fn draw_screen(lines: &[String], cx: usize, cy: usize, sy: usize, name: &str) {
    let mut w = WRITE_LOCK.lock();
    w.clear();
    
    // Top bar
    w.set_color(Color::Black, Color::LightGray);
    for _ in 0..COLS { w.write_byte(b' '); }
    let title = alloc::format!(" SparkOS Nano - Dosya: {} ", name);
    w.write_at(0, 0, &title, Color::Black, Color::LightGray);
    let help = " ESC: Kaydet ve Cik ";
    w.write_at(0, COLS - help.len(), help, Color::Red, Color::LightGray);

    // Text content
    w.set_color(Color::White, Color::Black);
    for row in 0..(ROWS - 1) {
        let line_idx = sy + row;
        if line_idx < lines.len() {
            let line = &lines[line_idx];
            w.write_at(row + 1, 0, line, Color::White, Color::Black);
        }
    }
    
    // Draw cursor indicator (inverted color)
    let screen_y = (cy - sy) + 1;
    if screen_y < ROWS {
        let ch = if cx < lines[cy].len() {
            lines[cy].as_bytes()[cx]
        } else {
            b' '
        };
        w.write_at(screen_y, cx, core::str::from_utf8(&[ch]).unwrap_or(" "), Color::Black, Color::White);
    }
}
