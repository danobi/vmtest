use std::env::consts::ARCH;
use std::ffi::OsString;
use std::fmt;
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use itertools::Itertools;
use log::{debug, log_enabled, warn, Level};
use qapi::{qga, qmp, Qga, Qmp};
use rand::Rng;

/// Represents a single QEMU instance
pub struct Qemu {
    process: Command,
    qga_sock: PathBuf,
    qmp_sock: PathBuf,
    command: String,
}

/// This struct contains the result of the qemu command execution.
///
/// The command could have succeeded or failed _inside_ the VM --
/// it is up for caller to interpret the contents of this struct.
#[derive(Default)]
pub struct QemuResult {
    /// Return code of command
    pub exitcode: i64,
    /// Stdout of command
    pub stdout: String,
    /// Stderr of command
    pub stderr: String,
}

const QEMU_DEFAULT_ARGS: &[&str] = &[
    "-nodefaults",
    "-display",
    "none",
    "-enable-kvm",
    "-m",
    "4G", // TODO(dxu): make configurable
    "-cpu",
    "host",
    "-smp",
    "2", // TOOD(dxu): make configurable
];

// Generate a path to a randomly named socket
fn gen_sock(prefix: &str) -> PathBuf {
    let mut path = PathBuf::new();
    path.push("/tmp");

    let id = rand::thread_rng().gen_range(100_000..1_000_000);
    let sock = format!("/tmp/{prefix}-{id}.sock");
    path.push(sock);

    path
}

/// Generate arguments for inserting a file as a drive into the guest
fn drive_args(file: &Path, index: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    args.push("-drive".into());

    let mut arg = OsString::new();
    arg.push("file=");
    arg.push(file);
    arg.push(",format=raw,index=");
    arg.push(index.to_string());
    arg.push(",media=disk,if=virtio,cache=none");
    args.push(arg);

    args
}

/// Generate arguments for setting up the guest agent on both host and guest
fn guest_agent_args(sock: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    args.push("-chardev".into());

    let mut arg = OsString::new();
    arg.push("socket,path=");
    arg.push(sock);
    arg.push(",server=on,wait=off,id=qga0");
    args.push(arg);

    args.push("-device".into());
    args.push("virtio-serial".into());

    args.push("-device".into());
    args.push("virtserialport,chardev=qga0,name=org.qemu.guest_agent.0".into());

    args
}

/// Generate arguments for setting up QMP control socket on host
fn machine_protocol_args(sock: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    args.push("-qmp".into());

    let mut arg = OsString::new();
    arg.push("unix:");
    arg.push(sock);
    arg.push(",server=on,wait=off");
    args.push(arg);

    args
}

/// Run a process inside the VM and wait until completion
///
/// NB: this is not a shell, so you won't get shell features unless you run a
/// `/bin/bash -c '...'`
fn run_in_vm<S>(qga: &mut Qga<S>, cmd: &str, args: &[&str]) -> Result<QemuResult>
where
    S: Write + BufRead,
{
    let qga_args = qga::guest_exec {
        path: cmd.to_string(),
        arg: Some(args.iter().map(|a| a.to_string()).collect()),
        capture_output: Some(true),
        input_data: None,
        env: None,
    };
    let handle = qga.execute(&qga_args).context("Failed to QGA guest-exec")?;
    let pid = handle.pid;

    let now = time::Instant::now();
    let mut period = Duration::from_millis(100);
    let status = loop {
        let status = qga
            .execute(&qga::guest_exec_status { pid })
            .context("Failed to QGA guest-exec-status")?;

        if status.exited {
            break status;
        }

        // Exponential backoff up to 5s so we don't poll too frequently
        if period <= (Duration::from_secs(5) / 2) {
            period *= 2;
        }

        let elapsed = now.elapsed();
        if now.elapsed() >= Duration::from_secs(30) {
            warn!(
                "'{cmd}' is taking a while to execute inside the VM ({}ms)",
                elapsed.as_secs()
            );
        }

        debug!("PID={pid} not finished; sleeping {}s", period.as_millis());
        thread::sleep(period);
    };

    let mut result = QemuResult::default();
    if let Some(code) = status.exitcode {
        result.exitcode = code;
    } else {
        warn!("Command '{cmd}' exitcode unknown");
    }
    if let Some(stdout) = status.out_data {
        result.stdout = String::from_utf8_lossy(&stdout).to_string();
    } else {
        debug!("Command '{cmd}' stdout missing")
    }
    if let Some(t) = status.out_truncated {
        if t {
            warn!("'{cmd}' stdout truncated");
        }
    }
    if let Some(stderr) = status.err_data {
        result.stderr = String::from_utf8_lossy(&stderr).to_string();
    } else {
        debug!("Command '{cmd}' stderr missing")
    }
    if let Some(t) = status.err_truncated {
        if t {
            warn!("'{cmd}' stderr truncated");
        }
    }

    Ok(result)
}

impl Qemu {
    /// Construct a QEMU instance backing a vmtest target.
    ///
    /// Does not run anything yet.
    pub fn new(image: &Path, kernel: Option<&Path>, command: &str) -> Self {
        let qga_sock = gen_sock("qga");
        let qmp_sock = gen_sock("qmp");

        let mut c = Command::new(format!("qemu-system-{}", ARCH));
        c.args(QEMU_DEFAULT_ARGS)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(machine_protocol_args(&qmp_sock))
            .args(guest_agent_args(&qga_sock))
            .args(drive_args(image, 1));

        if let Some(kernel) = kernel {
            c.arg("-kernel").arg(kernel);
        }

        if log_enabled!(Level::Debug) {
            let args = c.get_args().map(|a| a.to_string_lossy()).join(" ");
            debug!(
                "qemu invocation: {} {}",
                c.get_program().to_string_lossy(),
                args
            );
        }

        Self {
            process: c,
            qga_sock,
            qmp_sock,
            command: command.to_string(),
        }
    }

    /// Waits for QMP and QGA sockets to appear
    fn wait_for_qemu(&self, timeout: Option<Duration>) -> Result<()> {
        let now = time::Instant::now();
        let timeout = timeout.unwrap_or(Duration::from_secs(5));

        while now.elapsed() < timeout {
            let qga_ok = self
                .qga_sock
                .try_exists()
                .with_context(|| format!("Cannot stat {}", self.qga_sock.display()))?;

            let qmp_ok = self
                .qmp_sock
                .try_exists()
                .with_context(|| format!("Cannot stat {}", self.qga_sock.display()))?;

            if qga_ok && qmp_ok {
                return Ok(());
            }

            // The delay is usually quite small, so keep the retry interval low
            // to make vmtest appear snappy.
            thread::sleep(Duration::from_millis(50));
        }

        bail!("QEMU sockets did not appear in time");
    }

    /// Run this target's command inside the VM
    fn run_command<S>(&self, qga: &mut Qga<S>) -> Result<QemuResult>
    where
        S: Write + BufRead,
    {
        let parts = shell_words::split(&self.command).context("Failed to shell split command")?;
        // This is checked during config validation
        assert!(!parts.is_empty());

        let cmd = &parts[0];
        let args: Vec<&str> = parts
            .get(1..)
            .unwrap_or(&[])
            .iter()
            .map(|s| -> &str { s.as_ref() })
            .collect();

        run_in_vm(qga, cmd, &args)
    }

    /// Run the target to completion
    ///
    /// [`QemuResult`] is returned if command was successfully executed inside
    /// the VM (saying nothing about if the command was semantically successful).
    /// In other words, if the test harness was _able_ to execute the command,
    /// we return `QemuResult`. If the harness failed, we return error.
    pub fn run(mut self) -> Result<QemuResult> {
        let child = self.process.spawn().context("Failed to spawn process")?;
        // Ensure child is cleaned up even if we bail early
        let mut child = scopeguard::guard(child, |mut c| {
            match c.try_wait() {
                Ok(Some(e)) => debug!("Child already exited with {e}"),
                Ok(None) => {
                    // We must have bailed before we sent `quit` over QMP
                    debug!("Child still alive, killing");
                    if let Err(e) = c.kill() {
                        debug!("Failed to kill child: {}", e);
                    }
                    if let Err(e) = c.wait() {
                        debug!("Failed to wait on killed child: {}", e);
                    }
                }
                Err(e) => debug!("Failed to wait on child: {}", e),
            }
        });

        self.wait_for_qemu(None)
            .context("Failed waiting for QEMU to be ready")?;

        // Connect to QMP socket
        let qmp_stream = UnixStream::connect(&self.qmp_sock).context("Failed to connect QMP")?;
        let mut qmp = Qmp::from_stream(&qmp_stream);
        let qmp_info = qmp.handshake().context("QMP handshake failed")?;
        debug!("QMP info: {:#?}", qmp_info);

        // Connect to QGA socket
        let qga_stream = UnixStream::connect(&self.qga_sock).context("Failed to connect QGA")?;
        let mut qga = Qga::from_stream(&qga_stream);
        let sync_value = rand::thread_rng().gen_range(1..10_000);
        qga.guest_sync(sync_value)
            .context("Failed to QGA guest handshake")?;
        let qga_info = qga
            .execute(&qga::guest_info {})
            .context("Failed to get QGA info")?;
        debug!("QGA info: {:#?}", qga_info);

        // Run command in VM
        let qemu_result = self
            .run_command(&mut qga)
            .context("Failed to run command")?;

        // Quit and wait for QEMU to exit
        let _ = qmp.execute(&qmp::quit {}).context("Failed to QMP quit")?;
        let status = child.wait().context("Failed to wait on child")?;
        debug!("Exit code: {:?}", status.code());

        Ok(qemu_result)
    }
}

impl fmt::Display for QemuResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "\tExit code: {}", self.exitcode)?;
        writeln!(f, "\tStdout:\n {}", self.stdout)?;
        writeln!(f, "\tStderr:\n {}", self.stderr)?;

        Ok(())
    }
}
