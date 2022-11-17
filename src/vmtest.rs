use std::convert::AsRef;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::Config;

/// Central vmtest data structure
pub struct Vmtest {
    _base: PathBuf,
    _config: Config,
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
            if image.is_empty() {
                bail!("Target '{}' has empty image path", target.name);
            }
        }

        if let Some(kernel) = &target.kernel {
            if kernel.is_empty() {
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
    /// based off of. This is typically the directory the `Vmtest.toml` is
    /// found in.
    pub fn new<T: AsRef<Path>>(path: T, config: Config) -> Result<Self> {
        validate_config(&config).context("Invalid config")?;
        Ok(Self {
            _base: path.as_ref().to_owned(),
            _config: config,
        })
    }

    /// Run test matrix.
    pub fn run(&self) -> Result<()> {
        unimplemented!();
    }
}
