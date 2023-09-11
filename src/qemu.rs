use itertools::Itertools;
use std::collections::HashMap;
use std::env;
use std::env::consts::ARCH;
use std::ffi::{OsStr, OsString};
use std::hash::Hash;
use std::hash::Hasher;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;
use std::thread;
use std::time;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use log::{debug, log_enabled, warn, Level};
use qapi::{qga, qmp, Qmp};
use rand::Rng;
use tempfile::{Builder, NamedTempFile};

use crate::output::Output;
use crate::qga::QgaWrapper;
use crate::{Mount, VMConfig};

const INIT_SCRIPT: &str = include_str!("init/init.sh");
// Needs to be `/dev/root` for kernel to "find" the 9pfs as rootfs
const ROOTFS_9P_FS_MOUNT_TAG: &str = "/dev/root";
const SHARED_9P_FS_MOUNT_TAG: &str = "vmtest-shared";

const SHARED_9P_FS_MOUNT_PATH: &str = "/mnt/vmtest";
const MOUNT_OPTS_9P_FS: &str = "trans=virtio,cache=loose,msize=1048576";
const OVMF_PATHS: &[&str] = &[
    // Fedora
    "/usr/share/edk2/ovmf/OVMF_CODE.fd",
    // Ubuntu
    "/usr/share/OVMF/OVMF_CODE.fd",
    // Arch linux
    // TODO(dxu): parameterize by architecture
    "/usr/share/edk2-ovmf/x64/OVMF_CODE.fd",
];

/// Represents a single QEMU instance
pub struct Qemu {
    process: Command,
    qga_sock: PathBuf,
    qmp_sock: PathBuf,
    command: String,
    host_shared: PathBuf,
    mounts: HashMap<String, Mount>,
    _init: NamedTempFile,
    updates: Sender<Output>,
    /// Whether or not we are running an image target
    image: bool,
}

const QEMU_DEFAULT_ARGS: &[&str] = &["-nodefaults", "-display", "none"];

/// Whether or not the host supports KVM
fn host_supports_kvm() -> bool {
    Path::new("/dev/kvm").exists()
}

// Generate a path to a randomly named socket
fn gen_sock(prefix: &str) -> PathBuf {
    let mut path = PathBuf::new();
    path.push("/tmp");

    let id = rand::thread_rng().gen_range(100_000..1_000_000);
    let sock = format!("/tmp/{prefix}-{id}.sock");
    path.push(sock);

    path
}

fn gen_init() -> Result<NamedTempFile> {
    let mut f = Builder::new()
        .prefix("vmtest-init")
        .suffix(".sh")
        .rand_bytes(5)
        .tempfile()
        .context("Failed to create tempfile")?;

    f.write_all(INIT_SCRIPT.as_bytes())
        .context("Failed to write init to tmpfs")?;

    // Set write bits on script
    let mut perms = f
        .as_file()
        .metadata()
        .context("Failed to get init tempfile metadata")?
        .permissions();
    perms.set_mode(perms.mode() | 0o111);
    f.as_file()
        .set_permissions(perms)
        .context("Failed to set executable bits on init tempfile")?;

    Ok(f)
}

/// Generate arguments for inserting a file as a drive into the guest
fn drive_args(file: &Path, index: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    let disk_id = format!("disk{}", hash(file));
    args.push("-drive".into());
    args.push(
        format!(
            "file={},index={},media=disk,if=none,id={}",
            file.display(),
            index,
            disk_id
        )
        .into(),
    );
    args.push("-device".into());
    args.push(format!("virtio-blk-pci,drive={},bootindex={}", disk_id, index).into());

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

/// Generate arguments for full KVM virtualization if host supports it
fn kvm_args() -> Vec<&'static str> {
    let mut args = Vec::new();

    if host_supports_kvm() {
        args.push("-enable-kvm");
        args.push("-cpu");
        args.push("host");
    } else {
        args.push("-cpu");
        args.push("qemu64");
    }

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
///
/// `id` is the ID for the FS export (currently unused AFAICT)
/// `mount_tag` is used inside guest to find the export
fn plan9_fs_args(host_shared: &Path, id: &str, mount_tag: &str, ro: bool) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    args.push("-virtfs".into());

    let mut arg = OsString::new();
    arg.push(format!("local,id={id},path="));
    arg.push(if host_shared.as_os_str().is_empty() {
        // This case occurs when the config file path is just "vmtest.toml"
        Path::new(".")
    } else {
        host_shared
    });
    arg.push(format!(
        ",mount_tag={mount_tag},security_model=none,multidevs=remap"
    ));
    if ro {
        arg.push(",readonly=on")
    }
    args.push(arg);

    args
}

fn uefi_firmware_args(bios: Option<&Path>) -> Vec<OsString> {
    let mut args = Vec::new();

    args.push("-bios".into());

    if let Some(path) = bios {
        args.push(path.into());
        return args;
    }

    let mut chosen = OVMF_PATHS[0];
    for path in OVMF_PATHS {
        if Path::new(path).exists() {
            debug!("Found OVMF firmware: {}", path);
            chosen = path;
            break;
        }
    }
    args.push(chosen.into());

    args
}

/// Generate arguments for running a kernel with current userspace
///
/// The basic idea is we'll map host root onto guest root. And then use
/// the host's systemd as init but boot into `rescue.target` in the guest.
fn kernel_args(kernel: &Path, init: &Path, additional_kargs: Option<&String>) -> Vec<OsString> {
    let mut args = Vec::new();

    // Set the guest kernel
    args.push("-kernel".into());
    args.push(kernel.into());

    // See below `panic=-1` for explanation
    args.push("-no-reboot".into());

    // The guest kernel command line args
    let mut cmdline: Vec<OsString> = Vec::new();

    // Tell kernel the rootfs is 9p
    cmdline.push("rootfstype=9p".into());
    cmdline.push(format!("rootflags={}", MOUNT_OPTS_9P_FS).into());

    // Mount rootfs readable/writable to make experience more smooth.
    // Lots of tools expect to be able to write logs or change global
    // state. The user can override this setting by supplying an
    // additional `ro` kernel command line argument.
    cmdline.push("rw".into());

    // Show as much console output as we can bear
    cmdline.push("earlyprintk=serial,0,115200".into());
    // Disable userspace writing ratelimits
    cmdline.push("printk.devkmsg=on".into());
    cmdline.push("console=0,115200".into());
    cmdline.push("loglevel=7".into());

    // We are not using RAID and this will help speed up boot
    cmdline.push("raid=noautodetect".into());

    // Specify our custom init.
    //
    // Note we are assuming the host's tmpfs is attached to rootfs. Which
    // seems like a reasonable assumption.
    let mut init_arg = OsString::new();
    init_arg.push("init=");
    init_arg.push(init);
    cmdline.push(init_arg);

    // Trigger an immediate reboot on panic.
    // When paired with above `-no-reboot`, this will cause qemu to exit
    cmdline.push("panic=-1".into());

    // Append on additional kernel args
    if let Some(kargs) = additional_kargs {
        cmdline.extend(kargs.split_whitespace().map(|karg| OsStr::new(karg).into()));
    }

    // Set host side qemu kernel command line
    args.push("-append".into());
    args.push(cmdline.join(OsStr::new(" ")));

    args
}

fn hash<T: Hash + ?Sized>(s: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);

    h.finish()
}

fn vmconfig_args(vm: &VMConfig) -> Vec<OsString> {
    let mut args = vec![
        "-smp".into(),
        vm.num_cpus.to_string().into(),
        "-m".into(),
        vm.memory.clone().into(),
    ];

    for mount in vm.mounts.values() {
        let name = format!("mount{}", hash(&mount.host_path));
        args.append(&mut plan9_fs_args(
            &mount.host_path,
            &name,
            &name,
            !mount.writable,
        ));
    }

    let mut extra_args = vm
        .extra_args
        .clone()
        .into_iter()
        .map(|s: String| s.into())
        .collect::<Vec<OsString>>();
    args.append(&mut extra_args);

    // NOTE: bios handled in the UEFI code.

    args
}

/// Run a process inside the VM and wait until completion
///
/// NB: this is not a shell, so you won't get shell features unless you run a
/// `bash -c '...'`
///
/// `propagate_env` specifies if the calling environment should be propagated
/// into the VM. This is useful for running user specified commands which may
/// depend on the calling environment.
///
/// Returns the exit code if command is run
fn run_in_vm<F>(
    qga: &QgaWrapper,
    output: F,
    cmd: &str,
    args: &[&str],
    propagate_env: bool,
) -> Result<i64>
where
    F: Fn(String),
{
    let qga_args = qga::guest_exec {
        path: cmd.to_string(),
        arg: Some(args.iter().map(|a| a.to_string()).collect()),
        capture_output: Some(true),
        input_data: None,
        env: if propagate_env {
            Some(env::vars().map(|(k, v)| format!("{k}={v}")).collect())
        } else {
            None
        },
    };
    let handle = qga
        .guest_exec(qga_args)
        .context("Failed to QGA guest-exec")?;
    let pid = handle.pid;

    let now = time::Instant::now();
    let mut period = Duration::from_millis(200);
    let mut stdout_pos = 0;
    let mut stderr_pos = 0;
    let rc = loop {
        let status = qga
            .guest_exec_status(pid)
            .context("Failed to QGA guest-exec-status")?;

        // Give the most recent output to receiver
        if let Some(stdout) = status.out_data {
            String::from_utf8_lossy(&stdout)
                .lines()
                .skip(stdout_pos)
                .for_each(|line| {
                    output(line.to_string());
                    stdout_pos += 1;
                })
        }
        if let Some(t) = status.out_truncated {
            if t {
                output("<stdout truncation>".to_string());
            }
        }
        // Note we give stderr last as error messages are usually towards
        // the end of command output (if not the final line)
        if let Some(stderr) = status.err_data {
            String::from_utf8_lossy(&stderr)
                .lines()
                .skip(stderr_pos)
                .for_each(|line| {
                    output(line.to_string());
                    stderr_pos += 1;
                })
        }
        if let Some(t) = status.err_truncated {
            if t {
                output("<stderr truncation>".to_string());
            }
        }

        if status.exited {
            break status.exitcode.unwrap_or(0);
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

    Ok(rc)
}

impl Qemu {
    /// Construct a QEMU instance backing a vmtest target.
    ///
    /// Does not run anything yet.
    pub fn new(
        updates: Sender<Output>,
        image: Option<&Path>,
        kernel: Option<&Path>,
        kargs: Option<&String>,
        bios: Option<&Path>,
        command: &str,
        host_shared: &Path,
        uefi: bool,
        vm: &VMConfig,
    ) -> Result<Self> {
        let qga_sock = gen_sock("qga");
        let qmp_sock = gen_sock("qmp");
        let init = gen_init().context("Failed to generate init")?;

        let mut c = Command::new(format!("qemu-system-{}", ARCH));

        c.args(QEMU_DEFAULT_ARGS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("-serial")
            .arg("stdio")
            .args(kvm_args())
            .args(machine_protocol_args(&qmp_sock))
            .args(guest_agent_args(&qga_sock));
        // Always ensure the rootfs is first.
        if let Some(image) = image {
            c.args(drive_args(image, 1));
            if uefi {
                c.args(uefi_firmware_args(bios));
            }
        } else if let Some(kernel) = kernel {
            c.args(plan9_fs_args(
                Path::new("/"),
                "root",
                ROOTFS_9P_FS_MOUNT_TAG,
                false,
            ));
            c.args(kernel_args(kernel, init.path(), kargs));
        } else {
            panic!("Config validation should've enforced XOR");
        }
        // Now add the shared mount and other extra mounts.
        c.args(plan9_fs_args(
            host_shared,
            "shared",
            SHARED_9P_FS_MOUNT_TAG,
            false,
        ));
        c.args(vmconfig_args(vm));

        if log_enabled!(Level::Error) {
            let args = c.get_args().map(|a| a.to_string_lossy()).join(" ");
            debug!(
                "qemu invocation: {} {}",
                c.get_program().to_string_lossy(),
                args
            );
        }

        Ok(Self {
            process: c,
            qga_sock,
            qmp_sock,
            command: command.to_string(),
            host_shared: host_shared.to_owned(),
            mounts: vm.mounts.clone(),
            _init: init,
            updates,
            image: image.is_some(),
        })
    }

    /// Waits for QMP and QGA sockets to appear
    fn wait_for_qemu(&self) -> Result<()> {
        let now = time::Instant::now();
        let timeout = Duration::from_secs(5);

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

    /// Connect to QMP socket
    fn connect_to_qmp(&self) -> Result<UnixStream> {
        let now = time::Instant::now();
        let timeout = Duration::from_secs(5);

        while now.elapsed() < timeout {
            if let Ok(stream) = UnixStream::connect(&self.qmp_sock) {
                return Ok(stream);
            }

            // The delay is usually quite small, so keep the retry interval low
            // to make vmtest appear snappy.
            thread::sleep(Duration::from_millis(50));
        }

        // Run one final time to return the real error
        UnixStream::connect(&self.qmp_sock).map_err(|e| anyhow!(e))
    }

    /// Generates a bash script that runs `self.command`
    fn command_script(&self) -> String {
        if self.image {
            // Image targets do not share host FS at all, so a `cd` will never succeed
            self.command.clone()
        } else {
            // Kernel targets share host FS so this `cd` should succeed. If it doesn't,
            // the error is shown to the user as a hint.
            format!(
                "cd {}; {}",
                self.host_shared.to_string_lossy(),
                &self.command
            )
        }
    }

    /// Run this target's command inside the VM
    ///
    /// Note the command is run in a bash shell
    fn run_command(&self, qga: &QgaWrapper) -> Result<i64> {
        let output_fn = |line: String| {
            let _ = self.updates.send(Output::Command(line));
        };

        let cmd = "bash";
        let script = self.command_script();
        let args = ["-c", &script];

        // Note we are propagating environment variables for this command
        // only if it's a kernel target.
        run_in_vm(qga, output_fn, cmd, &args, !self.image)
    }

    /// Mount shared directory in the guest
    fn mount_in_guest(
        &self,
        qga: &QgaWrapper,
        guest_path: &str,
        mount_tag: &str,
        ro: bool,
    ) -> Result<()> {
        let output_fn = |line: String| {
            let _ = self.updates.send(Output::Setup(line));
        };

        let rc = run_in_vm(qga, output_fn, "mkdir", &["-p", guest_path], false)?;
        if rc != 0 {
            bail!("Failed to mkdir {}: exit code {}", guest_path, rc);
        }

        // We can race with VM/qemu coming up. So retry a few times with growing backoff.
        let mut rc = 0;
        for i in 0..5 {
            let mount_opts = if ro {
                format!("{},ro", MOUNT_OPTS_9P_FS)
            } else {
                MOUNT_OPTS_9P_FS.into()
            };
            rc = run_in_vm(
                qga,
                output_fn,
                "mount",
                &["-t", "9p", "-o", &mount_opts, mount_tag, guest_path],
                false,
            )?;

            // Exit code 32 from mount(1) indicates mount failure.
            // We want to retry in this case.
            if rc == 32 {
                thread::sleep(i * Duration::from_secs(1));
                continue;
            } else {
                break;
            }
        }
        if rc != 0 {
            bail!("Failed to mount {}: exit code {}", guest_path, rc);
        }

        Ok(())
    }

    /// Sync guest filesystems so any in-flight data has time to go out to host
    fn sync(&self, qga: &QgaWrapper) -> Result<()> {
        let rc = run_in_vm(qga, |_| {}, "sync", &[], false)?;
        if rc != 0 {
            bail!("Failed to sync guest filesystems: exit code {}", rc);
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

    /// Stream qemu stdout to the receiver.
    ///
    /// This typically contains the boot log which may be useful. Note
    /// we may generate "out of stage" output for the receiver. This is
    /// unfortunate but crucial, as kernel crashes still need to be
    /// reported.
    ///
    /// Calling this function will spawn a thread that takes ownership
    /// over the child's stdout and reads until the the process exits.
    fn stream_child_output(updates: Sender<Output>, child: &mut Child) {
        // unwrap() should never fail b/c we are capturing stdout
        let stdout = child.stdout.take().unwrap();
        let mut reader = BufReader::new(stdout);

        thread::spawn(move || {
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => break,
                    Ok(_) => {
                        // Remove newline
                        line.pop();
                        let _ = updates.send(Output::Boot(line));
                    }
                    Err(e) => debug!("Failed to read from qemu stdout: {}", e),
                };
            }
        });
    }

    /// Extracts stderr out from the child.
    ///
    /// Useful for when QEMU has errored out and we want to report the error
    /// back to the user.
    ///
    /// Any failures in extraction will be encoded into the return string.
    fn extract_child_stderr(child: &mut Child) -> String {
        let mut err = String::new();

        // unwrap() should never fail b/c we are capturing stdout
        let mut stderr = child.stderr.take().unwrap();
        if let Err(e) = stderr.read_to_string(&mut err) {
            err += &format!("<failed to read child stderr: {}>", e);
        }

        err
    }

    /// Run the target to completion
    ///
    /// Errors and return status are reported through the `updates` channel passed into the
    /// constructor.
    pub fn run(mut self) {
        let _ = self.updates.send(Output::BootStart);
        let mut child = match self.process.spawn() {
            Ok(c) => c,
            Err(e) => {
                let _ = self
                    .updates
                    .send(Output::BootEnd(Err(e).context("Failed to spawn QEMU")));
                return;
            }
        };
        Self::stream_child_output(self.updates.clone(), &mut child);
        // Ensure child is cleaned up even if we bail early
        let mut child = scopeguard::guard(child, Self::child_cleanup);

        if let Err(e) = self.wait_for_qemu() {
            let _ = self.updates.send(Output::BootEnd(
                Err(e).context("Failed waiting for QEMU to be ready"),
            ));
        }

        // Connect to QMP socket
        let qmp_stream = match self.connect_to_qmp() {
            Ok(s) => s,
            Err(e) => {
                let err = Self::extract_child_stderr(&mut child);
                let _ = self.updates.send(Output::BootEnd(
                    Err(e).context("Failed to connect QMP").context(err),
                ));
                return;
            }
        };
        let mut qmp = Qmp::from_stream(&qmp_stream);
        let qmp_info = match qmp.handshake() {
            Ok(i) => i,
            Err(e) => {
                let err = Self::extract_child_stderr(&mut child);
                let _ = self.updates.send(Output::BootEnd(
                    Err(e).context("QMP handshake failed").context(err),
                ));
                return;
            }
        };
        debug!("QMP info: {:#?}", qmp_info);

        // Connect to QGA socket
        let qga = QgaWrapper::new(self.qga_sock.clone(), host_supports_kvm());
        let qga = match qga {
            Ok(q) => q,
            Err(e) => {
                let _ = self
                    .updates
                    .send(Output::BootEnd(Err(e).context("Failed to connect QGA")));
                return;
            }
        };
        let _ = self.updates.send(Output::BootEnd(Ok(())));

        // Mount shared directory inside guest
        let _ = self.updates.send(Output::SetupStart);
        if let Err(e) =
            self.mount_in_guest(&qga, SHARED_9P_FS_MOUNT_PATH, SHARED_9P_FS_MOUNT_TAG, false)
        {
            let _ = self.updates.send(Output::SetupEnd(
                Err(e).context("Failed to mount shared directory in guest"),
            ));
            return;
        }
        for (guest_path, mount) in &self.mounts {
            if let Err(e) = self.mount_in_guest(
                &qga,
                guest_path,
                &format!("mount{}", hash(&mount.host_path)),
                !mount.writable,
            ) {
                let _ = self.updates.send(Output::SetupEnd(
                    Err(e).context(format!("Failed to mount {} in guest", guest_path)),
                ));
                return;
            }
        }
        let _ = self.updates.send(Output::SetupEnd(Ok(())));

        // Run command in VM
        let _ = self.updates.send(Output::CommandStart);
        match self.run_command(&qga) {
            Ok(rc) => {
                let _ = self.updates.send(Output::CommandEnd(Ok(rc)));
            }
            Err(e) => {
                let _ = self
                    .updates
                    .send(Output::CommandEnd(Err(e).context("Failed to run command")));
            }
        }

        if let Err(e) = self.sync(&qga) {
            warn!("Failed to sync filesystem: {}", e);
        }

        // Quit and wait for QEMU to exit
        match qmp.execute(&qmp::quit {}) {
            Ok(_) => match child.wait() {
                Ok(s) => debug!("Exit code: {:?}", s.code()),
                Err(e) => warn!("Failed to wait on child: {}", e),
            },
            // TODO(dxu): debug why we are getting errors here
            Err(e) => debug!("Failed to gracefull quit QEMU: {e}"),
        }
    }
}
