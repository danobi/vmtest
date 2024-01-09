use itertools::Itertools;
use std::collections::HashMap;
use std::env;
use std::env::consts::ARCH;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::{BufRead, BufReader, Read, Write};
use std::marker::Send;
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
use serde_derive::Serialize;
use tempfile::{Builder, NamedTempFile};
use tinytemplate::{format_unescaped, TinyTemplate};

use crate::output::Output;
use crate::qga::QgaWrapper;
use crate::{Mount, Target, VMConfig};

const INIT_SCRIPT: &str = include_str!("init/init.sh");
const COMMAND_TEMPLATE: &str = include_str!("init/command.template");
// Needs to be `/dev/root` for kernel to "find" the 9pfs as rootfs
const ROOTFS_9P_FS_MOUNT_TAG: &str = "/dev/root";
const SHARED_9P_FS_MOUNT_TAG: &str = "vmtest-shared";
const COMMAND_OUTPUT_PORT_NAME: &str = "org.qemu.virtio_serial.0";

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
    /// virtio-serial socket that streams command output
    command_sock: PathBuf,
    host_shared: PathBuf,
    /// Path to somewhere on the host that the guest should use as rootfs
    rootfs: PathBuf,
    arch: String,
    mounts: HashMap<String, Mount>,
    _init: NamedTempFile,
    updates: Sender<Output>,
    /// Whether or not we are running an image target
    image: bool,
}

/// Used by templating engine to render command
#[derive(Serialize)]
struct CommandContext {
    /// True if command should change working directory before executing.
    should_cd: bool,
    /// Path to directory shared between guest/host
    host_shared: PathBuf,
    /// User supplied command to run
    command: String,
    /// virtio-serial output port name
    command_output_port_name: String,
}

const QEMU_DEFAULT_ARGS: &[&str] = &["-nodefaults", "-display", "none"];

/// Whether or not the host supports KVM
fn host_supports_kvm(arch: &str) -> bool {
    arch == ARCH && Path::new("/dev/kvm").exists()
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

// Given a guest temp dir and a host init path, generate the path to the init file
// in the guest.
// This path is the one that will be passed to the guest via the kernel's `init=` parameter.
// Returns an error if the guest_temp_dir is not a suffix of host_init_path.parent().
fn guest_init_path(guest_temp_dir: PathBuf, host_init_path: PathBuf) -> Result<PathBuf> {
    // guest_temp_dir should be the suffix of host_init_path.parent()
    if !host_init_path
        .parent()
        .context(format!(
            "host_init_path {:?} should have a parent",
            host_init_path
        ))?
        .ends_with(guest_temp_dir.strip_prefix("/").context(format!(
            "guest_temp_dir {:?} should be an absolute path",
            guest_temp_dir
        ))?)
    {
        bail!(
            "guest_temp_dir {:?} should be a suffix of host_init_path.parent() {:?}",
            guest_temp_dir,
            host_init_path.parent()
        );
    }
    let mut guest_init_path = guest_temp_dir;
    guest_init_path.push(host_init_path.file_name().unwrap());
    Ok(guest_init_path)
}

// Given a rootfs, generate a tempfile with the init script inside.
// Returns the tempfile and the path to the init script inside the guest.
// When rootfs is /, both the tempfile filename and guest init path are equal.
// When rootfs is different than /, the guest init path is the same as the
// tempfile filename, but with the rootfs path stripped off.
fn gen_init(rootfs: &Path) -> Result<(NamedTempFile, PathBuf)> {
    let guest_temp_dir = std::env::temp_dir();
    let mut host_dest_dir = rootfs.to_path_buf().into_os_string();
    host_dest_dir.push(guest_temp_dir.clone().into_os_string());

    let mut host_init = Builder::new()
        .prefix("vmtest-init")
        .suffix(".sh")
        .rand_bytes(5)
        .tempfile_in::<OsString>(host_dest_dir)
        .context("Failed to create tempfile")?;

    host_init
        .write_all(INIT_SCRIPT.as_bytes())
        .context("Failed to write init to tmpfs")?;

    // Set write bits on script
    let mut perms = host_init
        .as_file()
        .metadata()
        .context("Failed to get init tempfile metadata")?
        .permissions();
    perms.set_mode(perms.mode() | 0o111);
    host_init
        .as_file()
        .set_permissions(perms)
        .context("Failed to set executable bits on init tempfile")?;

    // Path in the guest is our guest_temp_dir to which we append the file
    // name of the host init script.
    let guest_init = guest_init_path(guest_temp_dir, host_init.path().to_path_buf())?;
    debug!(
        "rootfs path: {rootfs:?}, init host path: {host_init:?}, init guest path: {guest_init:?}"
    );
    Ok((host_init, guest_init))
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
fn kvm_args(arch: &str) -> Vec<&'static str> {
    let mut args = Vec::new();

    if host_supports_kvm(arch) {
        args.push("-enable-kvm");
        args.push("-cpu");
        args.push("host");
    } else {
        args.push("-cpu");
        match arch {
            "aarch64" | "s390x" => {
                args.push("max");
            }
            _ => {
                args.push("qemu64");
            }
        }
    }
    args
}

/// Generate arguments for which qemu machine to use
fn machine_args(arch: &str) -> Vec<&'static str> {
    let mut args = Vec::new();

    if arch == "aarch64" {
        // aarch64 does not have default machines.
        args.push("-machine");
        args.push("virt,gic-version=3");
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

/// Generate which serial device to use based on the architecture used.
fn console_device(arch: &str) -> String {
    match arch {
        "aarch64" => "ttyAMA0".into(),
        _ => "0".into(),
    }
}
/// Generate arguments for running a kernel with current userspace
///
/// The basic idea is we'll map host root onto guest root. And then use
/// the host's systemd as init but boot into `rescue.target` in the guest.
fn kernel_args(
    kernel: &Path,
    arch: &str,
    init: &Path,
    additional_kargs: Option<&String>,
) -> Vec<OsString> {
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
    cmdline.push(format!("earlyprintk=serial,{},115200", console_device(arch)).into());
    // Disable userspace writing ratelimits
    cmdline.push("printk.devkmsg=on".into());
    cmdline.push(format!("console={},115200", console_device(arch)).into());
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

/// Generate arguments for setting up virtio-serial device to stream
/// command output from guest to host.
fn virtio_serial_args(host_sock: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();

    args.push("--device".into());
    args.push("virtio-serial".into());

    args.push("-chardev".into());
    let mut arg = OsString::new();
    arg.push("socket,path=");
    arg.push(host_sock);
    arg.push(",server=on,wait=off,id=cmdout");
    args.push(arg);

    args.push("--device".into());
    arg = OsString::new();
    arg.push("virtserialport,chardev=cmdout,name=");
    arg.push(COMMAND_OUTPUT_PORT_NAME);
    args.push(arg);

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

/// Stream command output to the output sink
///
/// Calling this function will spawn a thread that synchronously reads
/// from the provided unix domain socket. This is to reduce latency between
/// command output and text on screen (as opposed to non-blocking reads with
/// sleeps).
///
/// We implicitly rely on the stream closing (via synchronous qemu exit) to
/// terminate this thread.
fn stream_command_output<F>(stream: UnixStream, output: F)
where
    F: Fn(String) + Send + 'static,
{
    let mut reader = BufReader::new(stream);

    thread::spawn(move || {
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    // Remove newline
                    if let Some('\n') = line.chars().last() {
                        line.pop();
                    }
                    output(line);
                }
                Err(e) => debug!("Failed to read from command output stream: {}", e),
            };
        }
    });
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
/// `output_stream` is a unix domain socket that contains the streamed output
/// of `cmd`. Provide this when output latency is important (for example with
/// potentially long running `cmd`s).
///
/// Returns the exit code if command is run
fn run_in_vm<F>(
    qga: &QgaWrapper,
    output: &F,
    cmd: &str,
    args: &[&str],
    propagate_env: bool,
    output_stream: Option<UnixStream>,
) -> Result<i64>
where
    F: Fn(String) + Clone + Send + 'static,
{
    let version = qga.version();
    let qga_args = qga::guest_exec {
        path: cmd.to_string(),
        arg: Some(args.iter().map(|a| a.to_string()).collect()),
        // Merge stdout and stderr streams into stdout if qga supports it. Otherwise use
        // separate streams and process both.
        // Note this change is backwards compatible with older versions. The QAPI wire format
        // guarantees this.
        capture_output: Some(if version.major >= 8 && version.minor >= 1 {
            qga::GuestExecCaptureOutput::mode(qga::GuestExecCaptureOutputMode::merged)
        } else {
            qga::GuestExecCaptureOutput::flag(true)
        }),
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

    // If requested, start streaming output. We will still use guest-exec
    // output facilities as backup (and for streaming error messages).
    if let Some(stream) = output_stream {
        stream_command_output(stream, (*output).clone());
    }

    let now = time::Instant::now();
    let mut period = Duration::from_millis(200);
    let status = loop {
        let status = qga
            .guest_exec_status(pid)
            .context("Failed to QGA guest-exec-status")?;

        if status.exited {
            break status;
        }

        let elapsed = now.elapsed();
        if now.elapsed() >= Duration::from_secs(30) {
            warn!(
                "'{cmd}' is taking a while to execute inside the VM ({}ms)",
                elapsed.as_secs()
            );
        }

        debug!("PID={pid} not finished; sleeping {} ms", period.as_millis());
        thread::sleep(period);

        // Exponential backoff up to 5s so we don't poll too frequently
        if period <= (Duration::from_secs(5) / 2) {
            period *= 2;
        }
    };

    // Despite appearances, guest-exec-status only returns stdout and
    // stderr output _after_ the process exits. So parse it now after the
    // command is done.
    if let Some(stdout) = status.out_data {
        String::from_utf8_lossy(&stdout).lines().for_each(|line| {
            output(line.to_string());
        })
    }
    if let Some(true) = status.out_truncated {
        output("<stdout truncation>".to_string());
    }

    // Note we give stderr last as error messages are usually towards
    // the end of command output (if not the final line)
    if let Some(stderr) = status.err_data {
        String::from_utf8_lossy(&stderr).lines().for_each(|line| {
            output(line.to_string());
        })
    }
    if let Some(true) = status.err_truncated {
        output("<stderr truncation>".to_string());
    }

    Ok(status.exitcode.unwrap_or(0))
}

impl Qemu {
    /// Construct a QEMU instance backing a vmtest target.
    ///
    /// Does not run anything yet.
    pub fn new(updates: Sender<Output>, target: &Target, host_shared: &Path) -> Result<Self> {
        let qga_sock = gen_sock("qga");
        let qmp_sock = gen_sock("qmp");
        let command_sock = gen_sock("cmdout");
        let (init, guest_init) = gen_init(&target.rootfs).context("Failed to generate init")?;

        let mut c = Command::new(format!("qemu-system-{}", target.arch));

        c.args(QEMU_DEFAULT_ARGS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .arg("-serial")
            .arg("stdio")
            .args(kvm_args(&target.arch))
            .args(machine_args(&target.arch))
            .args(machine_protocol_args(&qmp_sock))
            .args(guest_agent_args(&qga_sock))
            .args(virtio_serial_args(&command_sock));
        // Always ensure the rootfs is first.
        if let Some(image) = target.image.clone() {
            c.args(drive_args(&image, 1));
            if target.uefi {
                c.args(uefi_firmware_args(target.vm.bios.as_deref()));
            }
        } else if let Some(kernel) = target.kernel.clone() {
            c.args(plan9_fs_args(
                target.rootfs.as_path(),
                "root",
                ROOTFS_9P_FS_MOUNT_TAG,
                false,
            ));
            c.args(kernel_args(
                &kernel,
                &target.arch,
                guest_init.as_path(),
                target.kernel_args.as_ref(),
            ));
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
        c.args(vmconfig_args(&target.vm));

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
            command: target.command.to_string(),
            command_sock,
            host_shared: host_shared.to_owned(),
            rootfs: target.rootfs.clone(),
            arch: target.arch.clone(),
            mounts: target.vm.mounts.clone(),
            _init: init,
            updates,
            image: target.image.is_some(),
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

    /// Connect to unix domain socket
    fn connect_to_uds(&self, path: &Path) -> Result<UnixStream> {
        let now = time::Instant::now();
        let timeout = Duration::from_secs(5);

        while now.elapsed() < timeout {
            if let Ok(stream) = UnixStream::connect(path) {
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
        // Disable HTML escaping (b/c we're not dealing with HTML)
        let mut tt = TinyTemplate::new();
        tt.set_default_formatter(&format_unescaped);

        // We are ok panicing here b/c there should never be a runtime
        // error compiling the template. Any errors are trivial bugs.
        tt.add_template("cmd", COMMAND_TEMPLATE).unwrap();

        let context = CommandContext {
            // Only `cd` for kernel targets that share userspace with host
            should_cd: !self.image && self.rootfs == Target::default_rootfs(),
            host_shared: self.host_shared.clone(),
            command: self.command.clone(),
            command_output_port_name: COMMAND_OUTPUT_PORT_NAME.into(),
        };

        // Same as above, ignore errors cuz only trivial bugs are possible
        tt.render("cmd", &context).unwrap()
    }

    /// Run this target's command inside the VM
    ///
    /// Note the command is run in a bash shell
    fn run_command(&self, qga: &QgaWrapper) -> Result<i64> {
        let updates = self.updates.clone();
        let output_fn = move |line: String| {
            let _ = updates.send(Output::Command(line));
        };

        let output_stream = self
            .connect_to_uds(&self.command_sock)
            .context("Failed to connect to command output socket")?;

        let cmd = "bash";
        let script = self.command_script();
        let args = ["-c", &script];

        // Note we are propagating environment variables for this command
        // only if it's a kernel target.
        run_in_vm(
            qga,
            &output_fn,
            cmd,
            &args,
            !self.image,
            Some(output_stream),
        )
    }

    /// Mount shared directory in the guest
    fn mount_in_guest(
        &self,
        qga: &QgaWrapper,
        guest_path: &str,
        mount_tag: &str,
        ro: bool,
    ) -> Result<()> {
        let updates = self.updates.clone();
        let output_fn = move |line: String| {
            let _ = updates.send(Output::Setup(line));
        };

        let rc = run_in_vm(qga, &output_fn, "mkdir", &["-p", guest_path], false, None)?;
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
                &output_fn,
                "mount",
                &["-t", "9p", "-o", &mount_opts, mount_tag, guest_path],
                false,
                None,
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
        let rc = run_in_vm(qga, &|_| {}, "sync", &[], false, None)?;
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
        let qmp_stream = match self.connect_to_uds(&self.qmp_sock) {
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
        let qga = QgaWrapper::new(self.qga_sock.clone(), host_supports_kvm(&self.arch));
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

impl Drop for Qemu {
    fn drop(&mut self) {
        let _ = fs::remove_file(self.qga_sock.as_path());
        let _ = fs::remove_file(self.qmp_sock.as_path());
        let _ = fs::remove_file(self.command_sock.as_path());
    }
}

#[cfg(test)]
mod tests {
    use super::guest_init_path;
    use rstest::rstest;

    use std::path::PathBuf;

    #[rstest]
    // no trailing /
    #[case("/tmp", "/foo/tmp/bar.sh", "/tmp/bar.sh")]
    // with trailing /
    #[case("/tmp/", "/foo/tmp/bar.sh", "/tmp/bar.sh")]
    // A valid case if rootfs was /foo/tmp and env::temp_dir() was /.
    #[case("/", "/foo/tmp/bar.sh", "/bar.sh")]
    // A valid case if env::temp_dir() was /foo/tmp and rootfs was /.
    #[case("/foo/tmp", "/foo/tmp/bar.sh", "/foo/tmp/bar.sh")]
    fn test_guest_init_path(
        #[case] guest_temp_dir: &str,
        #[case] host_init_path: &str,
        #[case] expected: &str,
    ) {
        let r = guest_init_path(guest_temp_dir.into(), host_init_path.into()).unwrap();
        assert_eq!(r, PathBuf::from(expected));
    }

    #[rstest]
    // This should never happen given that host_init_path is made by appending guest_temp_dir to rootfs.
    // for now it will return a guest_init_path which won't work.
    #[case("/tmp", "/foo/bar.sh")]
    // Invalid case because guest_temp_dir is not a suffix of dirname(host_init_path)
    #[case("/foo/tmp", "/bar/tmp/bar.sh")]
    fn test_invalid_guest_init_path(#[case] guest_temp_dir: &str, #[case] host_init_path: &str) {
        guest_init_path(guest_temp_dir.into(), host_init_path.into()).unwrap_err();
    }
}
