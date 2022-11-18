use std::env::consts::ARCH;
use std::ffi::OsString;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use itertools::Itertools;
use log::{debug, log_enabled, Level};
use qapi::{qga, qmp, Qga, Qmp};
use rand::Rng;

/// Represents a single QEMU instance
pub struct Qemu {
    process: Command,
    qga_sock: PathBuf,
    qmp_sock: PathBuf,
    _command: String,
}

const QEMU_DEFAULT_ARGS: &[&str] = &[
    "-nodefaults",
    "-display",
    "none",
    "-serial",
    "mon:stdio",
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

impl Qemu {
    /// Construct a QEMU instance backing a vmtest target.
    ///
    /// Does not run anything yet.
    pub fn new(image: &Path, kernel: Option<&Path>, command: &str) -> Self {
        let qga_sock = gen_sock("qga");
        let qmp_sock = gen_sock("qmp");

        let mut c = Command::new(format!("qemu-system-{}", ARCH));
        c.args(QEMU_DEFAULT_ARGS)
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
            _command: command.to_string(),
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

    /// Run the target to completion
    pub fn run(mut self) -> Result<()> {
        let mut child = self.process.spawn().context("Failed to spawn process")?;
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

        // Quit and wait for QEMU to exit
        let _ = qmp.execute(&qmp::quit {}).context("Failed to QMP quit")?;
        let status = child.wait().context("Failed to wait on child")?;
        debug!("Exit code: {:?}", status.code());

        Ok(())
    }
}
