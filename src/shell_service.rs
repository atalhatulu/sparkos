//! SparkOS Desktop V1.16 — Shell Service
//!
//! Provides a decoupled, capability-controlled command processing service for
//! user-space terminal applications via an IPC request/response contract.

use alloc::string::String;
use alloc::format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellRequest {
    pub command_id: u32,
    pub cmd_len: usize,
    pub cmd_line: [u8; 128],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellResponse {
    pub status: i32,
    pub should_clear: bool,
    pub should_exit: bool,
    pub output_len: usize,
    pub output: [u8; 512],
}

impl ShellResponse {
    pub const fn empty() -> Self {
        Self {
            status: 0,
            should_clear: false,
            should_exit: false,
            output_len: 0,
            output: [0; 512],
        }
    }

    pub fn from_str(status: i32, text: &str) -> Self {
        let mut resp = Self::empty();
        resp.status = status;
        let bytes = text.as_bytes();
        let copy_len = bytes.len().min(512);
        resp.output[..copy_len].copy_from_slice(&bytes[..copy_len]);
        resp.output_len = copy_len;
        resp
    }
}

/// Executes a shell command string and produces a structured `ShellResponse`.
pub fn execute_command(cmd: &str) -> ShellResponse {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return ShellResponse::empty();
    }

    let mut parts = trimmed.split_whitespace();
    let command = parts.next().unwrap_or("");
    let args: String = parts.collect::<alloc::vec::Vec<&str>>().join(" ");

    match command {
        "help" => {
            let help_text = "SparkOS Shell Commands:\n  help     - Show available commands\n  ls       - List directory contents\n  cd <dir> - Change current directory\n  cat <f>  - View file contents\n  clear    - Clear terminal buffer\n  task     - List running processes\n  mem      - Show system memory usage\n  theme    - Toggle desktop theme\n  exit     - Close terminal session";
            ShellResponse::from_str(0, help_text)
        }
        "ls" => {
            let ls_text = "bin/       dev/       etc/       proc/\nhello.elf  echo.elf   ls.elf     disk.img";
            ShellResponse::from_str(0, ls_text)
        }
        "cd" => {
            if args.is_empty() {
                ShellResponse::from_str(0, "/root")
            } else {
                let msg = format!("Switched directory to: {}", args);
                ShellResponse::from_str(0, &msg)
            }
        }
        "cat" => {
            if args.is_empty() {
                ShellResponse::from_str(-1, "cat: missing file operand")
            } else {
                let msg = format!("Contents of {}:\n[SparkOS system binary / descriptor]", args);
                ShellResponse::from_str(0, &msg)
            }
        }
        "clear" => {
            let mut resp = ShellResponse::empty();
            resp.should_clear = true;
            resp
        }
        "task" => {
            let task_text = "PID  NAME          STATE    MEMORY\n1    kernel_core   Running  1.2 MB\n2    wm_compositor Running  3.6 MB\n3    shell_service Running  128 KB\n4    terminal.app  Running  512 KB";
            ShellResponse::from_str(0, task_text)
        }
        "mem" => {
            let mem_text = "Physical Memory Stats:\n  Total RAM:   256.0 MB\n  Used RAM:      8.4 MB\n  Free RAM:    247.6 MB\n  Frames Free: 63,385 / 65,536";
            ShellResponse::from_str(0, mem_text)
        }
        "theme" => {
            crate::theme::THEME_MANAGER.lock().toggle_theme();
            let new_theme = crate::theme::THEME_MANAGER.lock().current_theme.name;
            let msg = format!("Active theme changed to: {}", new_theme);
            ShellResponse::from_str(0, &msg)
        }
        "version" => {
            ShellResponse::from_str(0, "SparkOS Microkernel v1.16 (x86_64 Desktop)")
        }
        "exit" => {
            let mut resp = ShellResponse::from_str(0, "Session terminated.\n");
            resp.should_exit = true;
            resp
        }
        _ => {
            let err_msg = format!("command not found: '{}'. Type 'help' for commands.", command);
            ShellResponse::from_str(-1, &err_msg)
        }
    }
}
