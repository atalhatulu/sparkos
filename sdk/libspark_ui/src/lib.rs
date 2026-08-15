//! libspark_ui — Native SparkOS User Interface Framework SDK
//!
//! Provides modular widgets, layouts, and event handling for SparkOS Ring-3 applications.

#![no_std]
extern crate alloc;

pub mod widget;
pub mod layout;

pub use widget::*;
pub use layout::*;
