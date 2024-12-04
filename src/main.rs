use std::cell::OnceCell;
use std::env::consts::ARCH;
use std::fs::{self, File};
use std::io::{stdout, IsTerminal as _};
use std::path::{Path, PathBuf};
use std::process::exit;
use std::{env, io};

use anyhow::{Context, Result};
use clap::Parser;
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
    /// Command to use to launch QEMU. Can be a full path or a PATH-resolved binary. If none is
    /// provided, we default to `qemu-system-$ARCH`, where $ARCH is the value of the `arch`
    /// argument
    #[clap(short, long, conflicts_with = "config")]
    qemu_command: Option<String>,
    /// Command to run in kernel mode. `-` to get an interactive shell.
    command: Vec<String>,
}

/// A type representing a log that creates the associated file lazily
/// upon first write.
#[derive(Default)]
struct DeferredLog {
    file: OnceCell<File>,
}

impl DeferredLog {
    fn file(&mut self) -> &File {
        self.file.get_or_init(|| {
            fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(".vmtest.log")
                // The `log` infrastructure would just swallow errors on
                // the regular write path and so we panic here to convey
                // any issues to users.
                .unwrap_or_else(|err| panic!("failed to create .vmtest.log: {err}"))
        })
    }
}

impl io::Write for DeferredLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file().flush()
    }
}

/// Initialize logging
///
/// This will send logs to a file named `.vmtest.log` if user is running
/// vmtest from a terminal. We do this so the logs don't get garbled by
/// the console manipulations from the UI. If not a terminal, simply print
/// to terminal and the logs will be inlined correctly.
fn init_logging() -> Result<()> {
    let target = match stdout().is_terminal() {
        false => LogTarget::Stderr,
        true => LogTarget::Pipe(Box::new(DeferredLog::default())),
    };

    Builder::from_default_env()
        .default_format()
        .target(target)
        .try_init()
        .context("Failed to init env_logger")?;

    Ok(())
}

/// Configure a `Vmtest` instance from command line arguments.
/// Filter out targets that don't match the provided regex.
/// Filtering is only applied when a config file is provided.
fn config(args: &Args) -> Result<Vmtest> {
    match &args.kernel {
        Some(kernel) => {
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
                    qemu_command: args.qemu_command.clone(),
                    command: args.command.join(" "),
                    vm: VMConfig::default(),
                }],
            };
            Vmtest::new(cwd, config)
        }
        None => {
            let default = Path::new("vmtest.toml").to_owned();
            let config_path = args.config.as_ref().unwrap_or(&default);
            let contents = fs::read_to_string(config_path).context("Failed to read config file")?;
            let filter = Regex::new(&args.filter).context("Failed to compile regex")?;
            let mut config: Config = toml::from_str(&contents).context("Failed to parse config")?;
            config.target = config
                .target
                .into_iter()
                .filter(|t| filter.is_match(&t.name))
                .map(|t| {
                    let mut t = t;
                    if !args.command.is_empty() {
                        t.command = args.command.join(" ");
                    }
                    t
                })
                .collect::<Vec<_>>();
            let base = config_path.parent().unwrap();
            Vmtest::new(base, config)
        }
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
    let ui = Ui::new(vmtest);
    let rc = ui.run(show_cmd(&args));

    exit(rc);
}

#[cfg(test)]
mod tests {

    use super::*;
    use tempfile::{Builder, TempDir};

    fn test_config() -> Result<TempDir> {
        let tmp_dir = Builder::new().tempdir()?;
        let config_path = tmp_dir.path().join("vmtest.toml");
        fs::write(
            &config_path,
            r#"
        [[target]]
        name = "test1"
        image = "test1.img"
        command = "echo test1"
        [[target]]
        name = "test2"
        kernel = "test2.kernel"
        command = "echo test2"
        "#,
        )
        .unwrap();
        Ok(tmp_dir)
    }

    #[test]
    fn test_config_no_filter() {
        let tmp_dir = test_config().expect("Failed to create config");
        let config_path = tmp_dir.path().join("vmtest.toml");

        let args = Args::parse_from([
            "cliname",
            "-c",
            config_path.to_str().expect("Failed to create config path"),
        ]);
        let vmtest = config(&args).expect("Failed to parse config");
        assert_eq!(vmtest.targets().len(), 2);
    }

    #[test]
    fn test_config_filter_match_all() {
        let tmp_dir = test_config().expect("Failed to create config");
        let config_path = tmp_dir.path().join("vmtest.toml");

        let args = Args::parse_from([
            "cliname",
            "-c",
            config_path.to_str().expect("Failed to create config path"),
            "-f",
            "test",
        ]);
        let vmtest = config(&args).expect("Failed to parse config");
        assert_eq!(vmtest.targets().len(), 2);
    }

    #[test]
    fn test_config_filter_match_last() {
        let tmp_dir = test_config().expect("Failed to create config");
        let config_path = tmp_dir.path().join("vmtest.toml");

        let args = Args::parse_from([
            "cliname",
            "-c",
            config_path.to_str().expect("Failed to create config path"),
            "-f",
            "test2",
        ]);
        let vmtest = config(&args).expect("Failed to parse config");
        assert_eq!(vmtest.targets().len(), 1);
        assert_eq!(vmtest.targets()[0].name, "test2");
    }

    // Test that when using the kernel argument, the filter is not applied.
    #[test]
    fn test_config_with_kernel_ignore_filter() {
        let args = Args::parse_from(["cliname", "-k", "mykernel", "-f", "test2", "command to run"]);
        let vmtest = config(&args).expect("Failed to parse config");
        assert_eq!(vmtest.targets().len(), 1);
        assert_eq!(vmtest.targets()[0].name, "mykernel");
    }
}
