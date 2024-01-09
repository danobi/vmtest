use std::collections::HashMap;
use std::env::consts::ARCH;
use std::path::PathBuf;
use std::vec::Vec;

use serde_derive::Deserialize;

/// Config for a mount
#[derive(Deserialize, Clone)]
pub struct Mount {
    /// Path on the host for the mount.
    pub host_path: PathBuf,
    /// Mount the location r/w.
    ///
    /// Default: false
    #[serde(default)]
    pub writable: bool,
}

/// VM Config for a target
#[derive(Deserialize, Clone)]
pub struct VMConfig {
    /// Number of CPUs in the VM.
    ///
    /// Default: 2
    #[serde(default = "VMConfig::default_cpus")]
    pub num_cpus: u8,
    /// Amount of RAM for the VM.
    ///
    /// Accepts a QEMU parsable string for the -m flag like 256M or 4G.
    /// Default: 4G
    #[serde(default = "VMConfig::default_memory")]
    pub memory: String,
    /// Map of additional Host mounts.
    ///
    /// Key is the path in the VM and the value is the path on the host.
    /// * Only respected when using an image.
    #[serde(default = "HashMap::new")]
    pub mounts: HashMap<String, Mount>,

    /// Path to the BIOS file.
    ///
    /// If this is empty, the default OS locations will be tried:
    /// * /usr/share/edk2/ovmf/OVMF_CODE.fd
    /// * /usr/share/OVMF/OVMF_CODE.fd
    /// * /usr/share/edk2-ovmf/x64/OVMF_CODE.fd
    pub bios: Option<PathBuf>,

    /// Extra arguments to pass to QEMU.
    #[serde(default = "Vec::new")]
    pub extra_args: Vec<String>,
    // TODO: Consider adding higher level interfaces for adding
    // additional hardware to the VM (USB, HDDs, CDROM, TPM, etc).
    // For now, people can use extra_args to add them.
}

impl VMConfig {
    fn default_cpus() -> u8 {
        2
    }

    fn default_memory() -> String {
        "4G".into()
    }
}

impl Default for VMConfig {
    fn default() -> Self {
        Self {
            num_cpus: Self::default_cpus(),
            memory: Self::default_memory(),
            mounts: HashMap::new(),
            bios: None,
            extra_args: Vec::new(),
        }
    }
}

/// Config for a single target
#[derive(Deserialize, Clone)]
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
    /// Path to rootfs to test against.
    ///
    /// * The path is relative to `vmtest.toml`.
    /// * If not specified, the host's rootfs will be used.
    /// Default: /
    #[serde(default = "Target::default_rootfs")]
    pub rootfs: PathBuf,
    /// Arch to run
    #[serde(default = "Target::default_arch")]
    pub arch: String,
    /// Command to run inside virtual machine.
    pub command: String,

    /// VM Configuration.
    #[serde(default)]
    pub vm: VMConfig,
}

impl Target {
    /// Default rootfs path to use if none are specified.
    pub fn default_rootfs() -> PathBuf {
        "/".into()
    }
    /// Default architecure to use if none are specified.
    pub fn default_arch() -> String {
        ARCH.to_string()
    }
}

impl Default for Target {
    fn default() -> Self {
        Self {
            name: "".into(),
            image: None,
            uefi: false,
            kernel: None,
            kernel_args: None,
            rootfs: Self::default_rootfs(),
            arch: Self::default_arch(),
            command: "".into(),
            vm: VMConfig::default(),
        }
    }
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

#[test]
fn test_default_vmconfig() {
    let config: Config = toml::from_str(
        r#"
        [[target]]
        name = "test"
        command = "real command"
        "#,
    )
    .unwrap();
    assert_eq!(config.target[0].vm.memory, "4G");
    assert_eq!(config.target[0].vm.num_cpus, 2);
    assert_eq!(config.target[0].vm.bios, None);
    assert_eq!(config.target[0].vm.extra_args.len(), 0);
    assert_eq!(config.target[0].vm.mounts.len(), 0);
}
