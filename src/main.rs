use std::fs;
use std::path::PathBuf;
use std::process::exit;

use anyhow::{Context, Result};
use clap::Parser;

use ::vmtest::{Ui, Vmtest};

#[derive(Parser, Debug)]
#[clap(version)]
struct Args {
    /// Path to config file
    #[clap(long, default_value = "vmtest.toml")]
    config: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::init();
    let contents = fs::read_to_string(&args.config).context("Failed to read config file")?;
    let config = toml::from_str(&contents).context("Failed to parse config")?;
    let base = args.config.parent().unwrap();
    let vmtest = Vmtest::new(base, config)?;
    let ui = Ui::new(vmtest);
    let failed = ui.run();
    let rc = if failed == 0 { 0 } else { 1 };

    exit(rc);
}
