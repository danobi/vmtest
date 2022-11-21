use std::env::consts::ARCH;
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use itertools::Itertools;
use log::{debug, error, log_enabled, warn, Level};
use qapi::{qga, qmp, Command as QapiCommand, Qga, Qmp};
use rand::Rng;

const SHARED_9P_FS_MOUNT_TAG: &str = "vmtest-shared";

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

/// Generate arguments for setting up 9p FS server on host
fn plan9_fs_args(host_shared: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    args.push("-virtfs".into());

    let mut arg = OsString::new();
    arg.push("local,id=shared,path=");
    arg.push(if host_shared.as_os_str().is_empty() {
        // This case occurs when the config file path is just "vmtest.toml"
        Path::new(".")
    } else {
        host_shared
    });
    arg.push(format!(
        ",mount_tag={SHARED_9P_FS_MOUNT_TAG},security_model=none"
    ));
    args.push(arg);

    args
}

/// Run a process inside the VM and wait until completion
///
/// NB: this is not a shell, so you won't get shell features unless you run a
/// `/bin/bash -c '...'`
fn run_in_vm(qga: &QgaWrapper, cmd: &str, args: &[&str]) -> Result<QemuResult> {
    let qga_args = qga::guest_exec {
        path: cmd.to_string(),
        arg: Some(args.iter().map(|a| a.to_string()).collect()),
        capture_output: Some(true),
        input_data: None,
        env: None,
    };
    let handle = qga
        .guest_exec(qga_args)
        .context("Failed to QGA guest-exec")?;
    let pid = handle.pid;

    let now = time::Instant::now();
    let mut period = Duration::from_millis(100);
    let status = loop {
        let status = qga
            .guest_exec_status(pid)
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
    pub fn new(
        image: &Path,
        kernel: Option<&Path>,
        command: &str,
        host_shared: &Path,
        uefi: bool,
    ) -> Self {
        let qga_sock = gen_sock("qga");
        let qmp_sock = gen_sock("qmp");

        let mut c = Command::new(format!("qemu-system-{}", ARCH));
        c.args(QEMU_DEFAULT_ARGS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("-serial")
            .arg("stdio")
            .args(machine_protocol_args(&qmp_sock))
            .args(guest_agent_args(&qga_sock))
            .args(plan9_fs_args(host_shared))
            .args(drive_args(image, 1));

        if let Some(kernel) = kernel {
            c.arg("-kernel").arg(kernel);
        }

        if uefi {
            c.arg("-bios").arg("/usr/share/edk2/ovmf/OVMF_CODE.fd");
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
    fn run_command(&self, qga: &QgaWrapper) -> Result<QemuResult> {
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

    /// Mount shared directory in the guest
    fn mount_shared(&self, qga: &QgaWrapper) -> Result<()> {
        let mkdir = run_in_vm(qga, "/bin/mkdir", &["-p", "/mnt/vmtest"])?;
        if mkdir.exitcode != 0 {
            bail!("Failed to mkdir /mnt/vmtest: {}", mkdir);
        }

        let msize = 1 << 20;
        let mount = run_in_vm(
            qga,
            "/bin/mount",
            &[
                "-t",
                "9p",
                "-o",
                &format!("trans=virtio,cache=loose,msize={msize}"),
                SHARED_9P_FS_MOUNT_TAG,
                "/mnt/vmtest",
            ],
        )?;
        if mount.exitcode != 0 {
            bail!("Failed to mount /mnt/vmtest: {}", mount);
        }

        Ok(())
    }

    /// Cleans up qemu child process if necessary
    fn child_cleanup(mut child: Child) {
        match child.try_wait() {
            Ok(Some(e)) => {
                debug!("Child already exited with {e}");
            }
            Ok(None) => {
                // We must have bailed before we sent `quit` over QMP
                debug!("Child still alive, killing");
                if let Err(e) = child.kill() {
                    debug!("Failed to kill child: {}", e);
                }
                if let Err(e) = child.wait() {
                    debug!("Failed to wait on killed child: {}", e);
                    return;
                }
            }
            Err(e) => {
                debug!("Failed to wait on child: {}", e);
                return;
            }
        }

        // Dump stdout/stderr in case it's useful for debugging
        if log_enabled!(Level::Debug) {
            if let Some(mut io) = child.stdout {
                let mut s = String::new();
                match io.read_to_string(&mut s) {
                    Ok(_) => debug!("qemu stdout: {s}"),
                    Err(e) => debug!("failed to get qemu stdout: {e}"),
                }
            }
            if let Some(mut io) = child.stderr {
                let mut s = String::new();
                match io.read_to_string(&mut s) {
                    Ok(_) => debug!("qemu stderr: {s}"),
                    Err(e) => debug!("failed to get qemu stderr: {e}"),
                }
            }
        }
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
        let mut child = scopeguard::guard(child, Self::child_cleanup);

        self.wait_for_qemu(None)
            .context("Failed waiting for QEMU to be ready")?;

        // Connect to QMP socket
        let qmp_stream = UnixStream::connect(&self.qmp_sock).context("Failed to connect QMP")?;
        let mut qmp = Qmp::from_stream(&qmp_stream);
        let qmp_info = qmp.handshake().context("QMP handshake failed")?;
        debug!("QMP info: {:#?}", qmp_info);

        // Connect to QGA socket
        let qga = QgaWrapper::new(self.qga_sock.clone()).context("Failed to connect QGA")?;

        // Mount shared directory inside guest
        self.mount_shared(&qga)
            .context("Failed to mount shared directory in guest")?;

        // Run command in VM
        let qemu_result = self.run_command(&qga).context("Failed to run command")?;

        // Quit and wait for QEMU to exit
        match qmp.execute(&qmp::quit {}) {
            Ok(_) => {
                let status = child.wait().context("Failed to wait on child")?;
                debug!("Exit code: {:?}", status.code());
            }
            // TODO(dxu): debug why we are getting errors here
            Err(e) => debug!("Failed to gracefull quit QEMU: {e}"),
        }

        Ok(qemu_result)
    }
}

impl fmt::Display for QemuResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Exit code: {}", self.exitcode)?;
        writeln!(f, "Stdout:\n {}", self.stdout)?;
        writeln!(f, "Stderr:\n {}", self.stderr)?;

        Ok(())
    }
}

/// This is a wrapper around [`Qga`] such that we can execute QGA commands
/// with a timeout.
///
/// The [`Qga`] has unapologetically blocking operations, meaning we can block
/// forever waiting for QGA to become ready in the guest. Instead, we'd like
/// to execute all commands with a timeout so we can provide a user friendly
/// error message if QGA never comes up in the guest.
struct QgaWrapper {
    send_req: Sender<QgaWrapperCommand>,
    recv_resp: Receiver<Result<QgaWrapperCommandResp>>,
}

#[allow(clippy::enum_variant_names)]
enum QgaWrapperCommand {
    GuestSync,
    GuestExec(qga::guest_exec),
    GuestExecStatus(qga::guest_exec_status),
}

#[allow(clippy::enum_variant_names)]
enum QgaWrapperCommandResp {
    GuestSync,
    GuestExec(<qga::guest_exec as QapiCommand>::Ok),
    GuestExecStatus(<qga::guest_exec_status as QapiCommand>::Ok),
}

impl QgaWrapper {
    fn new(sock: PathBuf) -> Result<Self> {
        let (send_req, recv_req) = mpsc::channel();
        let (send_resp, recv_resp) = mpsc::channel();

        // Start worker thread to service requests
        thread::spawn(move || Self::worker(sock, recv_req, send_resp));

        let r = Self {
            send_req,
            recv_resp,
        };

        r.guest_sync()?;

        Ok(r)
    }

    fn worker(
        sock: PathBuf,
        recv_req: Receiver<QgaWrapperCommand>,
        send_resp: Sender<Result<QgaWrapperCommandResp>>,
    ) {
        let qga_stream = match UnixStream::connect(sock) {
            Ok(s) => s,
            Err(e) => {
                // If we fail to connect to socket, exit this thread. The main
                // thread will detect a hangup and error accordingly.
                error!("Failed to connect QGA: {}", e);
                return;
            }
        };
        let mut qga = Qga::from_stream(&qga_stream);

        // We only get an error if other end hangs up. In the event of a hang up,
        // we gracefully terminate.
        while let Ok(req) = recv_req.recv() {
            let resp = match req {
                QgaWrapperCommand::GuestSync => {
                    let sync_value = rand::thread_rng().gen_range(1..10_000);
                    match qga.guest_sync(sync_value) {
                        Ok(_) => Ok(QgaWrapperCommandResp::GuestSync),
                        Err(e) => Err(anyhow!("Failed to guest_sync: {}", e)),
                    }
                }
                QgaWrapperCommand::GuestExec(args) => match qga.execute(&args) {
                    Ok(r) => Ok(QgaWrapperCommandResp::GuestExec(r)),
                    Err(e) => Err(anyhow!("Failed to guest_exec: {}", e)),
                },
                QgaWrapperCommand::GuestExecStatus(args) => match qga.execute(&args) {
                    Ok(r) => Ok(QgaWrapperCommandResp::GuestExecStatus(r)),
                    Err(e) => Err(anyhow!("Failed to guest_exec_status: {}", e)),
                },
            };

            // Note we do not care if this succeeds or not.
            // It is OK if receiver has gone away (eg we got timed out).
            let _ = send_resp.send(resp);
        }
    }

    fn execute(&self, cmd: QgaWrapperCommand, timeout: Duration) -> Result<QgaWrapperCommandResp> {
        match self.send_req.send(cmd) {
            Ok(_) => (),
            Err(e) => {
                debug!("Failed to send QGA command: {}", e);
                bail!("Failed to send QGA command: worker thread is dead")
            }
        };

        match self.recv_resp.recv_timeout(timeout) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => bail!("Timed out waiting for QGA"),
            Err(RecvTimeoutError::Disconnected) => {
                bail!("Failed to recv QGA command: worker thread is dead")
            }
        }
    }

    fn guest_sync(&self) -> Result<()> {
        self.execute(QgaWrapperCommand::GuestSync, Duration::from_secs(30))
            .map(|_| ())
    }

    fn guest_exec(&self, args: qga::guest_exec) -> Result<<qga::guest_exec as QapiCommand>::Ok> {
        match self.execute(QgaWrapperCommand::GuestExec(args), Duration::MAX)? {
            QgaWrapperCommandResp::GuestExec(resp) => Ok(resp),
            _ => panic!("Impossible return"),
        }
    }

    fn guest_exec_status(&self, pid: i64) -> Result<<qga::guest_exec_status as QapiCommand>::Ok> {
        match self.execute(
            QgaWrapperCommand::GuestExecStatus(qga::guest_exec_status { pid }),
            Duration::from_secs(5),
        )? {
            QgaWrapperCommandResp::GuestExecStatus(resp) => Ok(resp),
            _ => panic!("Impossible return"),
        }
    }
}
