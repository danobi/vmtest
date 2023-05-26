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
    /// * The path is relative to `vmtest.toml`.
    /// * The image must be bootable.
    pub image: Option<PathBuf>,
    /// Whether or not image should be booted using UEFI
    ///
    /// Default: false
    #[serde(default)]
    pub uefi: bool,
    /// Path to kernel image to test against.
    ///
    /// * The path is relative to `vmtest.toml`.
    /// * `vmlinux`, `vmlinuz`, and `bzImage` formats are accepted.
    pub kernel: Option<PathBuf>,
    /// Additional kernel command line parameters.
    ///
    /// Arguments are only valid for kernel targets.
    pub kernel_args: Option<String>,
    /// Command to run inside virtual machine.
    pub command: String,
}

/// Config containing full test matrix
#[derive(Deserialize)]
pub struct Config {
    /// List of targets in the testing matrix.
    pub target: Vec<Target>,
}

// Test that triple quoted toml strings are treated literally.
// This is used by vmtest-action to avoid escaping issues.
#[test]
fn test_triple_quoted_strings_are_literal() {
    let config: Config = toml::from_str(
        r#"
        [[target]]
        name = "test"
        command = '''this string has 'single' and "double" quotes'''
        "#,
    )
    .unwrap();

    assert_eq!(
        config.target[0].command,
        r#"this string has 'single' and "double" quotes"#
    );
}

// Similar to above, but test that backslash does not escape anything
#[test]
fn test_triple_quoted_strings_backslash() {
    let config: Config = toml::from_str(
        r#"
        [[target]]
        name = "test"
        command = '''this string has \back \slash\es'''
        "#,
    )
    .unwrap();

    assert_eq!(
        config.target[0].command,
        r#"this string has \back \slash\es"#
    );
}
