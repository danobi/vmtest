use std::env;
use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use vmtest::vmtest;

#[derive(Parser, Debug)]
#[clap(version)]
struct Args {
    /// Path to config file
    #[clap(long, default_value = "vmtest.toml")]
    config: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let contents = fs::read_to_string(&args.config).context("Failed to read config file")?;
    let config = toml::from_str(&contents).context("Failed to parse config")?;
    let cwd = env::current_dir().context("Failed to get current working directory")?;
    let vmtest = vmtest::Vmtest::new(cwd, config)?;
    vmtest.run().context("vmtest run failed")?;

    Ok(())
}
