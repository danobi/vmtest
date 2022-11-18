use std::convert::AsRef;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;
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

        if target.image.is_none() && target.kernel.is_none() {
            bail!(
                "Target '{}' must specify 'image', 'kernel', or both",
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

    /// Run test matrix.
    pub fn run(&self) -> Result<()> {
        // Run targets in serial
        //
        // TODO(dxu): run targets concurrently using async
        for target in &self.config.target {
            let resolved_image = self.resolve_path(target.image.as_deref());
            let resolved_kernel = self.resolve_path(target.kernel.as_deref());

            if let Some(image) = resolved_image {
                let qemu = Qemu::new(&image, resolved_kernel.as_deref(), &target.command);
                let result = qemu
                    .run()
                    .with_context(|| format!("Failed to run target '{}'", target.name))?;

                println!("Target '{}' results:", target.name);
                println!("{}", result);
            } else {
                bail!("Target '{}': image is currently required", target.name)
            }
        }

        Ok(())
    }
}
