#![deny(missing_docs)]
//! Library form of vmtest

/// Vmtest configuration.
pub mod config;
mod qemu;
mod qga;
/// Contains main vmtest logic.
pub mod vmtest;
