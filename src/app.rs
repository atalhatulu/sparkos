//! App initialization and lifecycle management for SparkOS.
//!
//! # User Application Contract
//! - **Entry Point**: The application must provide a valid ELF64 entry point.
//! - **Syscalls**: Accessible via `int 0x80`. See `sysapi.rs` for available syscalls.
//! - **Memory Layout**: 
//!   - Code and Data segments are loaded as defined in the ELF header.
//!   - A 4KB stack is automatically allocated for the application.
//!   - All memory allocated for the user app is mapped as User Accessible (Ring 3).

use crate::user::exec_elf_async;
use crate::serial_println;

/// The lifecycle state of a user application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Init,
    Running,
    Exit(u64),
    Error(&'static str),
}

/// Runs an ELF binary in Ring 3 userspace as an async executor task.
///
/// Delegates to `exec_elf_async`, which yields the cooperative executor while
/// the user app runs, so other kernel tasks (timer / IPC / input) keep making
/// progress instead of freezing during application execution. Returns the app
/// lifecycle result once the app exits or fails to load.
#[allow(dead_code)]
pub async fn run_app(elf_bytes: &[u8]) -> AppState {
    serial_println!("[APP] Initializing user application...");
    
    match exec_elf_async(elf_bytes).await {
        Ok(_) => {
            serial_println!("[APP] Application terminated successfully.");
            AppState::Exit(0)
        }
        Err(e) => {
            serial_println!("[APP] Application error: {}", e);
            AppState::Error(e)
        }
    }
}
