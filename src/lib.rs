#![deny(missing_docs)]
//! Library form of vmtest

/// Vmtest configuration.
pub mod config;
/// Contains user interface code.
pub mod ui;
/// Contains main vmtest logic.
pub mod vmtest;

pub use crate::config::*;
pub use crate::ui::*;
pub use crate::vmtest::*;

mod output;
mod qemu;
mod qga;
