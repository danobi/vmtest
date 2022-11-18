#![deny(missing_docs)]
//! Library form of vmtest

/// Vmtest configuration.
pub mod config;
mod qemu;
/// Contains main vmtest logic.
pub mod vmtest;
