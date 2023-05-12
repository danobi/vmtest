use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use anyhow::{Context, Result};
use clap::Parser;
use regex::Regex;

use ::vmtest::{Config, Target, Ui, Vmtest};

#[derive(Parser, Debug)]
#[clap(version)]
struct Args {
    /// Path to config file
    #[clap(short, long)]
    config: Option<PathBuf>,
    /// Filter by regex which targets to run
    ///
    /// This option takes a regular expression. If a target matches this regular
    /// expression, the target will be run.
    ///
    /// Supported regex syntax: https://docs.rs/regex/latest/regex/#syntax.
    #[clap(short, long, default_value = ".*")]
    filter: String,
    #[clap(short, long, conflicts_with = "config")]
    kernel: Option<PathBuf>,
    #[clap(conflicts_with = "config")]
    command: Vec<String>,
}

/// Configure a `Vmtest` instance from command line arguments.
fn config(args: &Args) -> Result<Vmtest> {
    if let Some(kernel) = &args.kernel {
        let cwd = env::current_dir().context("Failed to get current directory")?;
        let config = Config {
            target: vec![Target {
                name: kernel.file_name().unwrap().to_string_lossy().to_string(),
                image: None,
                uefi: false,
                kernel: Some(kernel.clone()),
                kernel_args: None,
                command: args.command.join(" "),
            }],
        };

        Vmtest::new(cwd, config)
    } else {
        let default = Path::new("vmtest.toml").to_owned();
        let config_path = args.config.as_ref().unwrap_or(&default);
        let contents = fs::read_to_string(config_path).context("Failed to read config file")?;
        let config = toml::from_str(&contents).context("Failed to parse config")?;
        let base = config_path.parent().unwrap();

        Vmtest::new(base, config)
    }
}

fn main() -> Result<()> {
    let args = Args::parse();

    env_logger::init();
    let vmtest = config(&args)?;
    let filter = Regex::new(&args.filter).context("Failed to compile regex")?;
    let ui = Ui::new(vmtest);
    let failed = ui.run(&filter);
    let rc = i32::from(failed != 0);

    exit(rc);
}
