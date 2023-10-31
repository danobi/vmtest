use std::convert::AsRef;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{Config, Target};
use crate::output::Output;
use crate::qemu::Qemu;

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

        if !target.uefi && target.vm.bios.is_some() {
            bail!(
                "Target '{}' cannot specify a bios without setting 'uefi'",
                target.name
            );
        }

        if target.kernel_args.is_some() && target.kernel.is_none() {
            bail!(
                "Target '{}' must specify 'kernel' with 'kernel_args'",
                target.name
            );
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
    fn resolve_path(&self, input: &Path) -> PathBuf {
        if input.is_relative() {
            let mut r = self.base.clone();
            r.push(input);

            r
        } else {
            input.to_owned()
        }
    }

    /// Returns registered targets
    pub fn targets(&self) -> &[Target] {
        &self.config.target
    }

    /// Setups up a `Qemu` instance for a run
    fn setup_qemu(&self, idx: usize, updates: Sender<Output>) -> Result<Qemu> {
        let mut target: Target = self
            .config
            .target
            .get(idx)
            .ok_or_else(|| anyhow!("idx={} out of range", idx))?
            .clone();
        target.image = target.image.map(|s| self.resolve_path(s.as_path()));
        target.kernel = target.kernel.map(|s| self.resolve_path(s.as_path()));
        target.rootfs = self.resolve_path(target.rootfs.as_path());
        target.vm.bios = target.vm.bios.map(|s| self.resolve_path(s.as_path()));

        Qemu::new(updates, &target, &self.base).context("Failed to setup QEMU")
    }

    /// Run a single target
    ///
    /// `idx` is the position of the target in the target list (0-indexed).
    ///
    /// `updates` is the channel real time updates should be sent to. See
    /// [`Output`] docs for more details.
    pub fn run_one(&self, idx: usize, updates: Sender<Output>) {
        match self.setup_qemu(idx, updates.clone()) {
            Ok(q) => q.run(),
            Err(e) => {
                let _ = updates.send(Output::BootEnd(Err(e)));
            }
        };
    }
}
