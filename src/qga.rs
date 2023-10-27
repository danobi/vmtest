use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use log::{debug, error, info, warn};
use qapi::{qga, Command as QapiCommand, Qga};
use rand::Rng;

const KVM_TIMEOUT: Duration = Duration::from_secs(60);
const EMULATE_TIMEOUT: Duration = Duration::from_secs(120);

/// This is a wrapper around [`Qga`] such that we can execute QGA commands
/// with a timeout.
///
/// The [`Qga`] has unapologetically blocking operations, meaning we can block
/// forever waiting for QGA to become ready in the guest. Instead, we'd like
/// to execute all commands with a timeout so we can provide a user friendly
/// error message if QGA never comes up in the guest.
pub struct QgaWrapper {
    stream: UnixStream,
    // Version of guest agent
    version: Version,
}

/// Version triple
#[derive(Default, Clone)]
pub struct Version {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

impl Version {
    fn new(s: &str) -> Result<Self> {
        let err_f = || anyhow!("Failed to parse version string '{}'", s);

        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.len() != 3 {
            bail!(err_f());
        }

        Ok(Version {
            major: u8::from_str(parts[0]).with_context(err_f)?,
            minor: u8::from_str(parts[1]).with_context(err_f)?,
            patch: u8::from_str(parts[2]).with_context(err_f)?,
        })
    }
}

impl QgaWrapper {
    /// Create a new `QgaWrapper`
    ///
    /// `sock` is the path to the QGA socket.
    /// `has_kvm` whether or not host supports KVM
    pub fn new(sock: PathBuf, has_kvm: bool) -> Result<Self> {
        let timeout = if has_kvm {
            KVM_TIMEOUT
        } else {
            EMULATE_TIMEOUT
        };

        // If we try reading the socket too  early, we'll hang forever and never run the test.
        // So do the guest_sync first with a timeout to ensure that the VM Guest Agent is up.
        let end = Instant::now() + timeout;
        let mut i = 0;
        while Instant::now() < end {
            info!("Connecting to QGA ({i})");
            i += 1;
            let qga_stream = match UnixStream::connect(&sock) {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to connect QGA, retrying: {}", e);
                    thread::sleep(Duration::from_secs(1));
                    continue;
                }
            };
            qga_stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            let mut qga = Qga::from_stream(&qga_stream);
            let sync_value = rand::thread_rng().gen_range(1..10_000);
            match qga.guest_sync(sync_value) {
                Ok(_) => {
                    let version = qga.execute(&qga::guest_info {})?.version;
                    debug!("qga version: {}", version);
                    return Ok(Self {
                        stream: qga_stream,
                        version: Version::new(&version)?,
                    });
                }
                Err(e) => {
                    warn!("QGA sync failed, retrying: {e}");
                    thread::sleep(Duration::from_secs(1));
                }
            }
        }

        bail!("Timed out waiting for QGA connection");
    }

    /// Run a command inside the guest
    pub fn guest_exec(
        &self,
        args: qga::guest_exec,
    ) -> Result<<qga::guest_exec as QapiCommand>::Ok> {
        let mut qga = Qga::from_stream(&self.stream);
        qga.execute(&args).context("Error running guest_exec")
    }

    /// Query status of a command inside the guest
    pub fn guest_exec_status(
        &self,
        pid: i64,
    ) -> Result<<qga::guest_exec_status as QapiCommand>::Ok> {
        let mut qga = Qga::from_stream(&self.stream);
        qga.execute(&qga::guest_exec_status { pid })
            .context("error running guest_exec_status")
    }

    /// Version triple of the guest agent (in the guest of course)
    pub fn version(&self) -> Version {
        self.version.clone()
    }
}
