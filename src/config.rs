use std::path::PathBuf;
use std::vec::Vec;

use serde_derive::Deserialize;

/// Config for a single target
#[derive(Deserialize)]
pub struct Target {
    /// Name of the testing target.
    pub name: String,
    /// Path to image to test against.
    ///
    /// * The path is relative to `Vmtest.toml`.
    /// * The image must be bootable.
    pub image: Option<PathBuf>,
    /// Path to kernel image to test against.
    ///
    /// * The path is relative to `Vmtest.toml`.
    /// * `vmlinux`, `vmlinuz`, and `bzImage` formats are accepted.
    pub kernel: Option<PathBuf>,
    /// Command to run inside virtual machine.
    pub command: String,
}

/// Config containing full test matrix
#[derive(Deserialize)]
pub struct Config {
    /// List of targets in the testing matrix.
    pub target: Vec<Target>,
}
