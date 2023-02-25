use std::fs;
use std::path::PathBuf;
use std::process::exit;

use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;

use ::vmtest::{Ui, Vmtest};

#[derive(Parser, Debug)]
#[clap(version)]
struct Args {
    /// Path to config file
    #[clap(long, default_value = "vmtest.toml")]
    config: PathBuf,
    /// Filter by regex which targets to run
    ///
    /// This option takes a regular expression. If a target matches this regular
    /// expression, the target will be run.
    ///
    /// Supported regex syntax: https://docs.rs/regex/latest/regex/#syntax.
    #[clap(long, default_value = ".*")]
    filter: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::init();
    let contents = fs::read_to_string(&args.config).context("Failed to read config file")?;
    let config = toml::from_str(&contents).context("Failed to parse config")?;
    let base = args.config.parent().unwrap();
    let vmtest = Vmtest::new(base, config)?;
    let filter = Regex::new(&args.filter).context("Failed to compile regex")?;
    let ui = Ui::new(vmtest);
    let failed = ui.run(&filter);
    let rc = i32::from(failed != 0);

    exit(rc);
}
