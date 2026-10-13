//! SparkOS Desktop V1.21 — Text Layout & Paragraph Engine
//!
//! Provides multi-line text wrapping, line truncation with ellipsis, and coordinate mapping.

use alloc::string::String;
use alloc::vec::Vec;

pub struct TextLayout;

impl TextLayout {
    /// Wraps text into lines that fit within `max_width` in pixels (assuming 8px glyph width)
    pub fn wrap_text(text: &str, max_width: u32) -> Vec<String> {
        let max_chars_per_line = (max_width / 8).max(1) as usize;
        let mut lines = Vec::new();

        for paragraph in text.split('\n') {
            let mut cur_line = String::new();
            for word in paragraph.split_whitespace() {
                if cur_line.is_empty() {
                    cur_line.push_str(word);
                } else if cur_line.len() + 1 + word.len() <= max_chars_per_line {
                    cur_line.push(' ');
                    cur_line.push_str(word);
                } else {
                    lines.push(cur_line);
                    cur_line = String::from(word);
                }
            }
            if !cur_line.is_empty() || paragraph.is_empty() {
                lines.push(cur_line);
            }
        }
        lines
    }

    /// Truncates text with an ellipsis if it exceeds `max_width`
    pub fn truncate_ellipsis(text: &str, max_width: u32) -> String {
        let max_chars = (max_width / 8) as usize;
        if text.len() <= max_chars {
            String::from(text)
        } else if max_chars > 3 {
            let mut s = String::from(&text[..max_chars - 3]);
            s.push_str("...");
            s
        } else {
            String::from(&text[..max_chars])
        }
    }
}
