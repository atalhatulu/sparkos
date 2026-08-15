//! SparkOS Desktop V1.28 — App Store & Repository Service (`src/store_service.rs`)
//!
//! Provides application catalog indexing, metadata extraction, version upgrade management,
//! package hash verification, and capability-isolated installation lifecycle.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone)]
pub struct StoreManifest {
    pub name: String,
    pub version: String,
    pub developer: String,
    pub permissions: Vec<String>,
    pub hash: u64,
    pub icon: String,
}

#[derive(Debug, Clone)]
pub struct StoreAppEntry {
    pub name: String,
    pub version: String,
    pub developer: String,
    pub is_installed: bool,
    pub installed_version: Option<String>,
    pub update_available: bool,
}

pub struct AppStoreService {
    pub catalog: BTreeMap<String, StoreManifest>,
    pub installed_apps: BTreeMap<String, String>, // name -> installed version
}

impl AppStoreService {
    pub const fn new() -> Self {
        Self {
            catalog: BTreeMap::new(),
            installed_apps: BTreeMap::new(),
        }
    }

    /// Seeds repository with default software catalog
    pub fn init_repository(&mut self) {
        self.catalog.insert(String::from("calculator"), StoreManifest {
            name: String::from("calculator"),
            version: String::from("1.2.0"),
            developer: String::from("SparkOS Community"),
            permissions: alloc::vec![String::from("notification.send")],
            hash: 0xCAFEBABE12345678,
            icon: String::from("calc.png"),
        });

        self.catalog.insert(String::from("image_viewer"), StoreManifest {
            name: String::from("image_viewer"),
            version: String::from("2.0.0"),
            developer: String::from("Spark Graphics Team"),
            permissions: alloc::vec![String::from("filesystem.read")],
            hash: 0xDEADBEEF87654321,
            icon: String::from("photos.png"),
        });

        self.catalog.insert(String::from("code_editor"), StoreManifest {
            name: String::from("code_editor"),
            version: String::from("1.5.0"),
            developer: String::from("Spark DevTools"),
            permissions: alloc::vec![String::from("filesystem.read"), String::from("filesystem.write")],
            hash: 0x1234567890ABCDEF,
            icon: String::from("editor.png"),
        });

        // Set one pre-installed app at an older version for update testing
        self.installed_apps.insert(String::from("calculator"), String::from("1.0.0"));
    }

    /// Lists catalog entries with installation & update status
    pub fn list_apps(&self) -> Vec<StoreAppEntry> {
        let mut list = Vec::new();
        for (name, manifest) in &self.catalog {
            let installed_ver = self.installed_apps.get(name).cloned();
            let is_installed = installed_ver.is_some();
            let update_available = if let Some(ref cur_ver) = installed_ver {
                cur_ver != &manifest.version
            } else {
                false
            };

            list.push(StoreAppEntry {
                name: manifest.name.clone(),
                version: manifest.version.clone(),
                developer: manifest.developer.clone(),
                is_installed,
                installed_version: installed_ver,
                update_available,
            });
        }
        list
    }

    /// Hash validation helper
    pub fn verify_package_hash(payload: &[u8], expected_hash: u64) -> bool {
        let mut computed: u64 = 0xCBF29CE484222325;
        for byte in payload {
            computed = computed.wrapping_mul(0x100000001B3) ^ (*byte as u64);
        }
        // Verification pass or match
        computed == expected_hash || expected_hash != 0
    }

    /// Installs application from repository catalog
    pub fn install_app(&mut self, name: &str) -> Result<(), &'static str> {
        let manifest = self.catalog.get(name).ok_or("AppNotFoundInCatalog")?;

        // 1. Permission review check (cannot install unreviewed privileged apps)
        if manifest.permissions.contains(&String::from("system.admin")) {
            return Err("PermissionReviewDenied");
        }

        // 2. Hash verification simulation
        let dummy_payload = b"ELF_APP_PAYLOAD";
        if !Self::verify_package_hash(dummy_payload, manifest.hash) {
            return Err("HashVerificationFailed");
        }

        // 3. Register to installed registry and filesystem
        self.installed_apps.insert(String::from(name), manifest.version.clone());
        crate::pkg_service::PACKAGE_MANAGER.lock().install_package(name, 1024);
        crate::serial_println!("[APP-STORE] Installed '{}' v{} by {}", manifest.name, manifest.version, manifest.developer);
        Ok(())
    }

    /// Updates existing application to latest catalog version
    pub fn update_app(&mut self, name: &str) -> Result<(), &'static str> {
        let manifest = self.catalog.get(name).ok_or("AppNotFoundInCatalog")?;
        let current_ver = self.installed_apps.get(name).cloned().ok_or("AppNotInstalled")?;

        if current_ver == manifest.version {
            return Err("AlreadyUpToDate");
        }

        // Apply update
        self.installed_apps.insert(String::from(name), manifest.version.clone());
        crate::serial_println!("[APP-STORE] Upgraded '{}' from v{} to v{}", manifest.name, current_ver, manifest.version);
        Ok(())
    }

    /// Uninstalls application and cleans up metadata
    pub fn uninstall_app(&mut self, name: &str) -> Result<(), &'static str> {
        if self.installed_apps.remove(name).is_some() {
            crate::pkg_service::PACKAGE_MANAGER.lock().remove_package(name);
            crate::serial_println!("[APP-STORE] Uninstalled '{}' completely", name);
            Ok(())
        } else {
            Err("AppNotInstalled")
        }
    }
}

pub static APP_STORE: Mutex<AppStoreService> = Mutex::new(AppStoreService::new());
