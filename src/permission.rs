//! SparkOS Desktop V1.19 — Application Permission & Manifest Management System
//!
//! Provides Android/macOS-style capability-based permission gating from `app.json` manifests,
//! enforcing strict capability filtering on application launch without microkernel pollution.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPermission {
    FilesystemRead,
    FilesystemWrite,
    NetworkAccess,
    CameraAccess,
    DeviceAccess,
    NotificationSend,
}

impl AppPermission {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim() {
            "filesystem.read" => Some(Self::FilesystemRead),
            "filesystem.write" => Some(Self::FilesystemWrite),
            "network" => Some(Self::NetworkAccess),
            "camera" => Some(Self::CameraAccess),
            "device" => Some(Self::DeviceAccess),
            "notification.send" => Some(Self::NotificationSend),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FilesystemRead => "filesystem.read",
            Self::FilesystemWrite => "filesystem.write",
            Self::NetworkAccess => "network",
            Self::CameraAccess => "camera",
            Self::DeviceAccess => "device",
            Self::NotificationSend => "notification.send",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppManifest {
    pub name: String,
    pub permissions: Vec<AppPermission>,
}

impl AppManifest {
    pub fn new(name: &str, permissions: Vec<AppPermission>) -> Self {
        Self {
            name: String::from(name),
            permissions,
        }
    }

    /// Simple deterministic parser for app.json manifest strings
    pub fn parse_manifest(json_text: &str) -> Result<Self, &'static str> {
        let mut name = String::from("UnknownApp");
        let mut permissions = Vec::new();

        // Extract "name"
        if let Some(pos) = json_text.find("\"name\"") {
            let rest = &json_text[pos..];
            if let Some(colon) = rest.find(':') {
                let after_colon = &rest[colon + 1..];
                if let Some(start_quote) = after_colon.find('"') {
                    let val_rest = &after_colon[start_quote + 1..];
                    if let Some(end_quote) = val_rest.find('"') {
                        name = String::from(&val_rest[..end_quote]);
                    }
                }
            }
        }

        // Extract permissions
        for candidate in ["filesystem.read", "filesystem.write", "network", "camera", "device", "notification.send"] {
            if json_text.contains(candidate) {
                if let Some(p) = AppPermission::from_str(candidate) {
                    if !permissions.contains(&p) {
                        permissions.push(p);
                    }
                }
            }
        }

        Ok(Self { name, permissions })
    }
}

pub struct PermissionManager {
    pub granted: BTreeMap<u64, Vec<AppPermission>>,
}

impl PermissionManager {
    pub const fn new() -> Self {
        Self {
            granted: BTreeMap::new(),
        }
    }

    pub fn register_process_permissions(&mut self, pid: u64, manifest: &AppManifest) {
        self.granted.insert(pid, manifest.permissions.clone());
        crate::serial_println!("[PERM-MGR] Granted {} permissions to PID {} ('{}')",
            manifest.permissions.len(), pid, manifest.name);
    }

    pub fn unregister_process(&mut self, pid: u64) {
        self.granted.remove(&pid);
        crate::serial_println!("[PERM-MGR] Cleared permissions for PID {}", pid);
    }

    pub fn check_permission(&self, pid: u64, perm: AppPermission) -> Result<(), &'static str> {
        if let Some(perms) = self.granted.get(&pid) {
            if perms.contains(&perm) {
                return Ok(());
            }
        }
        crate::serial_println!("[PERM-MGR] Access denied: PID {} lacks permission '{:?}'", pid, perm);
        Err("PermissionDenied")
    }
}

pub static PERMISSION_MANAGER: Mutex<PermissionManager> = Mutex::new(PermissionManager::new());
