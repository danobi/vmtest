use std::convert::AsRef;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::config::Config;
use crate::qemu::{Qemu, QemuResult};

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

        if target.image.is_none() && target.kernel.is_none() {
            bail!(
                "Target '{}' must specify 'image', 'kernel', or both",
                target.name
            );
        }

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
    /// `idx` is the position of the target in the target list (0-indexed)
    pub fn run_one(&self, idx: usize) -> Result<QemuResult> {
        let target = self
            .config
            .target
            .get(idx)
            .ok_or_else(|| anyhow!("idx={} out of range", idx))?;
        let image = self
            .resolve_path(target.image.as_deref())
            .ok_or_else(|| anyhow!("Target '{}': image is currently required", target.name))?;
        let kernel = self.resolve_path(target.kernel.as_deref());

        Qemu::new(
            &image,
            kernel.as_deref(),
            &target.command,
            &self.base,
            target.uefi,
        )
        .run()
        .context("Failed to run QEMU")
    }

    /// Convenience wrapper to run entire test matrix
    ///
    /// Note this method prints results to stdout/stderr
    pub fn run(&self) -> Result<()> {
        let mut failed = 0;
        for (idx, target) in self.config.target.iter().enumerate() {
            let title = format!("Target '{}' results:", target.name);
            println!("{}", title);
            println!("{}", "=".repeat(title.len()));

            match self.run_one(idx) {
                Ok(result) => {
                    println!("{}", result);
                    if result.exitcode != 0 {
                        failed += 1;
                    }
                }
                Err(e) => {
                    // NB: need to use debug formatting to get full error chain
                    eprintln!("Failed to run: {:?}", e);
                    failed += 1;
                }
            };
        }

        if failed > 0 {
            bail!("{} targets failed", failed);
        }

        Ok(())
    }
}
