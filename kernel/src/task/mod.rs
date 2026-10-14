use alloc::string::String;
use alloc::collections::BTreeMap;
use spin::Mutex;

pub mod keyboard;
pub mod process;

pub static PROCESS_LIST: spin::Lazy<Mutex<BTreeMap<u64, String>>> = spin::Lazy::new(|| Mutex::new(BTreeMap::new()));
pub static KILLED_PROCESSES: spin::Lazy<crate::sync::IrqMutex<alloc::vec::Vec<u64>>> = spin::Lazy::new(|| crate::sync::IrqMutex::new(alloc::vec::Vec::new()));
