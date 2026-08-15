//! SparkOS — User Authentication, Multi-User Sessions & Security Subsystem (Faz 23)
//!
//! Provides Ring-3 Authentication Daemon (authsvc), Salted Password Verification,
//! Multi-User Session Management, POSIX InodeV2 Permission Enforcement, and
//! Brute-Force Lockout Defense.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use spin::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    UserNotFound,
    InvalidPassword,
    AccountLocked,
    SessionNotFound,
    PermissionDenied,
}

#[derive(Debug, Clone)]
pub struct UserRecord {
    pub uid: u16,
    pub gid: u16,
    pub username: String,
    pub home_dir: String,
    pub shell: String,
}

#[derive(Debug, Clone)]
pub struct ShadowRecord {
    pub username: String,
    pub salt: [u8; 16],
    pub password_hash: [u8; 32],
    pub failed_attempts: u8,
    pub locked: bool,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub session_id: u32,
    pub uid: u16,
    pub gid: u16,
    pub username: String,
    pub pids: Vec<u64>,
    pub active: bool,
}

pub struct AuthManager {
    pub users: BTreeMap<String, UserRecord>,
    pub shadow: BTreeMap<String, ShadowRecord>,
    pub sessions: BTreeMap<u32, Session>,
    pub next_session_id: u32,
}

impl AuthManager {
    pub const fn new() -> Self {
        Self {
            users: BTreeMap::new(),
            shadow: BTreeMap::new(),
            sessions: BTreeMap::new(),
            next_session_id: 1,
        }
    }

    /// Computes deterministic salted SHA-256 simulation hash over password + salt.
    pub fn compute_hash(password: &[u8], salt: &[u8; 16]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut h: u64 = 0xcbf29ce484222325; // FNV-1a 64-bit basis
        for &b in password {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        for &b in salt {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }

        let bytes = h.to_be_bytes();
        for i in 0..32 {
            out[i] = bytes[i % 8] ^ (i as u8);
        }
        out
    }

    /// Registers a new user with initial credentials.
    pub fn add_user(&mut self, username: &str, uid: u16, gid: u16, password: &[u8]) {
        let user = UserRecord {
            uid,
            gid,
            username: String::from(username),
            home_dir: String::from(if uid == 0 { "/root" } else { "/home/user" }),
            shell: String::from("/bin/sh"),
        };

        // Deterministic salt generation
        let mut salt = [0u8; 16];
        for (i, b) in username.as_bytes().iter().enumerate() {
            if i < 16 {
                salt[i] = *b ^ 0xAA;
            }
        }

        let password_hash = Self::compute_hash(password, &salt);
        let shadow = ShadowRecord {
            username: String::from(username),
            salt,
            password_hash,
            failed_attempts: 0,
            locked: false,
        };

        self.users.insert(String::from(username), user);
        self.shadow.insert(String::from(username), shadow);
    }

    /// Authenticates user credentials, managing failed attempt lockout and creating a session.
    pub fn authenticate(&mut self, username: &str, password: &[u8]) -> Result<u32, AuthError> {
        let shadow = self.shadow.get_mut(username).ok_or(AuthError::UserNotFound)?;

        if shadow.locked {
            return Err(AuthError::AccountLocked);
        }

        let candidate_hash = Self::compute_hash(password, &shadow.salt);

        if candidate_hash != shadow.password_hash {
            shadow.failed_attempts += 1;
            if shadow.failed_attempts >= 3 {
                shadow.locked = true;
            }
            return Err(AuthError::InvalidPassword);
        }

        // Reset failed attempts on success
        shadow.failed_attempts = 0;

        let user = self.users.get(username).ok_or(AuthError::UserNotFound)?;
        let session_id = self.next_session_id;
        self.next_session_id += 1;

        let session = Session {
            session_id,
            uid: user.uid,
            gid: user.gid,
            username: String::from(username),
            pids: Vec::new(),
            active: true,
        };

        self.sessions.insert(session_id, session);
        Ok(session_id)
    }

    /// Terminates an active user session and invalidates its state.
    pub fn logout(&mut self, session_id: u32) -> Result<(), AuthError> {
        let session = self.sessions.get_mut(&session_id).ok_or(AuthError::SessionNotFound)?;
        session.active = false;
        session.pids.clear();
        self.sessions.remove(&session_id);
        Ok(())
    }

    /// Evaluates POSIX permission bits (rwxr-xr-x) for a given user against an Inode's metadata.
    pub fn check_posix_permission(
        uid: u16,
        gid: u16,
        inode_uid: u16,
        inode_gid: u16,
        mode: u16,
        requested_right: u8, // 4: Read, 2: Write, 1: Execute
    ) -> bool {
        // Root (UID 0) has implicit read/write POSIX access
        if uid == 0 {
            return true;
        }

        let shift = if uid == inode_uid {
            6 // Owner bits (bits 8..6)
        } else if gid == inode_gid {
            3 // Group bits (bits 5..3)
        } else {
            0 // Other bits (bits 2..0)
        };

        let perm = ((mode >> shift) & 0x7) as u8;
        (perm & requested_right) == requested_right
    }
}

pub static AUTH_MANAGER: Mutex<AuthManager> = Mutex::new(AuthManager::new());
