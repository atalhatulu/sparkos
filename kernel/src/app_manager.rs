//! SparkOS Desktop V1.9 — Desktop Application Lifecycle Service
//!
//! Tracks process lifecycle states (Created, Running, Minimized, Background, Closing, Terminated),
//! window bindings, and enforces clean resource reclaim on termination.

use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    Created,
    Running,
    Minimized,
    Background,
    Closing,
    Terminated,
}

#[derive(Debug, Clone)]
pub struct ManagedApp {
    pub pid: u64,
    pub name: String,
    pub window_id: Option<u64>,
    pub surface_id: Option<u64>,
    pub state: AppState,
    pub memory_bytes: usize,
}

pub struct ApplicationManager {
    pub apps: Vec<ManagedApp>,
}

impl ApplicationManager {
    pub const fn new() -> Self {
        Self { apps: Vec::new() }
    }

    pub fn register_app(&mut self, pid: u64, name: &str, window_id: Option<u64>, surface_id: Option<u64>) {
        self.apps.push(ManagedApp {
            pid,
            name: String::from(name),
            window_id,
            surface_id,
            state: AppState::Running,
            memory_bytes: 4096 * 4,
        });
        crate::serial_println!("[APP-MGR] Registered application '{}' (PID {}) -> State: Running", name, pid);
    }

    pub fn set_state(&mut self, pid: u64, state: AppState) {
        if let Some(app) = self.apps.iter_mut().find(|a| a.pid == pid) {
            app.state = state;
            crate::serial_println!("[APP-MGR] PID {} state transition -> {:?}", pid, state);
        }
    }

    pub fn terminate_app(&mut self, pid: u64) -> Result<(), &'static str> {
        if let Some(pos) = self.apps.iter().position(|a| a.pid == pid) {
            let app = &self.apps[pos];
            if let Some(wid) = app.window_id {
                let _ = crate::wm::WM.lock().destroy_window(pid, wid);
            }
            if let Some(sid) = app.surface_id {
                let _ = crate::surface::destroy_surface(sid);
            }
            crate::input::cleanup_input_for_pid(pid);
            self.apps.remove(pos);
            crate::serial_println!("[APP-MGR] Terminated application PID {} and cleaned all resources", pid);
            Ok(())
        } else {
            Err("AppNotFound")
        }
    }
}

pub static APP_MANAGER: Mutex<ApplicationManager> = Mutex::new(ApplicationManager::new());
