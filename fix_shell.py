with open('src/shell.rs', 'r') as f:
    shell = f.read()

# Add swap_buffers after prompt
shell = shell.replace('core::fmt::Write::write_str(&mut *w, "> ").unwrap();', 'core::fmt::Write::write_str(&mut *w, "> ").unwrap();\n        crate::gui::swap_buffers();')

# Add swap_buffers after writing a char
shell = shell.replace('core::fmt::Write::write_char(&mut *w, c as char).unwrap();', 'core::fmt::Write::write_char(&mut *w, c as char).unwrap();\n                            crate::gui::swap_buffers();')

# Add swap_buffers after Enter
shell = shell.replace('core::fmt::Write::write_str(&mut *w, "\\n").unwrap();', 'core::fmt::Write::write_str(&mut *w, "\\n").unwrap();\n                        crate::gui::swap_buffers();')

# Add swap_buffers at the end of exec
shell = shell.replace('w.set_color(self.text_color, Color::Black);\n    }', 'w.set_color(self.text_color, Color::Black);\n        crate::gui::swap_buffers();\n    }')

# Add swap_buffers after Backspace and Escape handling
shell = shell.replace('crate::gui::draw_rect(px, py, 8, 8, w.bg_color); // Tamamen sil', 'crate::gui::draw_rect(px, py, 8, 8, w.bg_color); // Tamamen sil\n                                crate::gui::swap_buffers();')
shell = shell.replace('self.len = 0;\n                    }', 'self.len = 0;\n                        crate::gui::swap_buffers();\n                    }')

with open('src/shell.rs', 'w') as f:
    f.write(shell)
print("Shell fixed!")
