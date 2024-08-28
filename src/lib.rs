#![deny(missing_docs)]
//! Library form of vmtest

/// Vmtest configuration.
pub mod config;
/// Contains definitions for streaming output
pub mod output;
/// Contains user interface code.
pub mod ui;
/// Contains main vmtest logic.
pub mod vmtest;

pub use crate::config::*;
pub use crate::ui::*;
pub use crate::vmtest::*;

mod qemu;
mod qga;
mod util;
