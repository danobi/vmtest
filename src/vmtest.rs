use std::convert::AsRef;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::Config;
use crate::qemu::{Output, Qemu, QemuResult};

/// Central vmtest data structure
pub struct Vmtest {
    base: PathBuf,
    config: Config,
}

/// Validate the statically known config parameters
fn validate_config(config: &Config) -> Result<()> {
    for (idx, target) in config.target.iter().enumerate() {
        if target.name.is_empty() {
            bail!("Target index={} name empty", idx);
        }

        // Must choose image XOR kernel. We do not allow combining image and kernel
        // b/c images typically make use of initramfs to locate the root disk,
        // handle encrypted partitions, LVM, etc., and we cannot accurately guess
        // how to handle boot. Nor can we place a kernel _in_ the image.
        //
        // So we force user to choose one or the other. If sufficiently motivated,
        // the user can always install the kernel into the image and use vmtest
        // in image mode.
        match (&target.image, &target.kernel) {
            (None, None) => bail!("Target '{}' must specify 'image' or 'kernel'", target.name),
            (Some(_), Some(_)) => bail!(
                "Target '{}' specified both 'image' and 'kernel'",
                target.name
            ),
            _ => (),
        };

        if target.uefi && target.image.is_none() {
            bail!("Target '{}' must specify 'image' with 'uefi'", target.name);
        }

        if let Some(image) = &target.image {
            if image.as_os_str().is_empty() {
                bail!("Target '{}' has empty image path", target.name);
            }
        }

        if let Some(kernel) = &target.kernel {
            if kernel.as_os_str().is_empty() {
                bail!("Target '{}' has empty kernel path", target.name);
            }
        }

        if target.command.is_empty() {
            bail!("Target '{}' has empty command", target.name);
        }
    }

    Ok(())
}

impl Vmtest {
    /// Construct a new instance.
    ///
    /// `path` is the working directory all relative config paths should be
    /// based off of. This is typically the directory the `vmtest.toml` is
    /// found in.
    pub fn new<T: AsRef<Path>>(path: T, config: Config) -> Result<Self> {
        validate_config(&config).context("Invalid config")?;
        Ok(Self {
            base: path.as_ref().to_owned(),
            config,
        })
    }

    /// Resolve an input path relative to the base path
    fn resolve_path(&self, input: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = input {
            if p.is_relative() {
                let mut r = self.base.clone();
                r.push(p);

                Some(r)
            } else {
                Some(p.to_owned())
            }
        } else {
            None
        }
    }

    /// Run a single target
    ///
    /// `idx` is the position of the target in the target list (0-indexed).
    ///
    /// `updates` is the channel real time updates should be sent to. See
    /// [`Output`] docs for more details.
    pub fn run_one(&self, idx: usize, updates: Sender<Output>) -> Result<QemuResult> {
        let target = self
            .config
            .target
            .get(idx)
            .ok_or_else(|| anyhow!("idx={} out of range", idx))?;
        let image = self.resolve_path(target.image.as_deref());
        let kernel = self.resolve_path(target.kernel.as_deref());

        Qemu::new(
            updates,
            image.as_deref(),
            kernel.as_deref(),
            &target.command,
            &self.base,
            target.uefi,
        )
        .context("Failed to setup QEMU")?
        .run()
        .context("Failed to run QEMU")
    }
}
