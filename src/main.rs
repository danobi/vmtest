use std::env;
use std::env::consts::ARCH;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::exit;

use anyhow::{Context, Result};
use clap::Parser;
use console::user_attended;
use env_logger::{fmt::Target as LogTarget, Builder};
use regex::Regex;

use vmtest::{Config, Target, Ui, VMConfig, Vmtest};

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
    /// Kernel to run
    #[clap(short, long, conflicts_with = "config")]
    kernel: Option<PathBuf>,
    /// Additional kernel command line arguments
    #[clap(long, conflicts_with = "config")]
    kargs: Option<String>,
    /// Location of rootfs, default to host's /
    #[clap(short, long, conflicts_with = "config", default_value = Target::default_rootfs().into_os_string())]
    rootfs: PathBuf,
    /// Arch to run
    #[clap(short, long, default_value = ARCH, conflicts_with = "config")]
    arch: String,
    #[clap(conflicts_with = "config")]
    command: Vec<String>,
}

/// Initialize logging
///
/// This will send logs to a file named `.vmtest.log` if user is running
/// vmtest from a terminal. We do this so the logs don't get garbled by
/// the console manipulations from the UI. If not a terminal, simply print
/// to terminal and the logs will be inlined correctly.
fn init_logging() -> Result<()> {
    let target = match user_attended() {
        false => LogTarget::Stderr,
        true => {
            let file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(".vmtest.log")
                .context("Failed to open log file")?;
            LogTarget::Pipe(Box::new(file))
        }
    };

    Builder::from_default_env()
        .default_format()
        .target(target)
        .try_init()
        .context("Failed to init env_logger")?;

    Ok(())
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
                rootfs: args.rootfs.clone(),
                arch: args.arch.clone(),
                kernel_args: args.kargs.clone(),
                command: args.command.join(" "),
                vm: VMConfig::default(),
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

/// Whether or not to collapse command output in UI.
///
/// This is useful for one-liner invocations.
fn show_cmd(args: &Args) -> bool {
    args.config.is_none()
}

fn main() -> Result<()> {
    let args = Args::parse();

    init_logging().context("Failed to initialize logging")?;
    let vmtest = config(&args)?;
    let filter = Regex::new(&args.filter).context("Failed to compile regex")?;
    let ui = Ui::new(vmtest);
    let rc = ui.run(&filter, show_cmd(&args));

    exit(rc);
}
