use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Result};
use log::{debug, error};
use qapi::{qga, Command as QapiCommand, Qga};
use rand::Rng;

const KVM_TIMEOUT: Duration = Duration::from_secs(30);
const EMULATE_TIMEOUT: Duration = Duration::from_secs(120);

/// This is a wrapper around [`Qga`] such that we can execute QGA commands
/// with a timeout.
///
/// The [`Qga`] has unapologetically blocking operations, meaning we can block
/// forever waiting for QGA to become ready in the guest. Instead, we'd like
/// to execute all commands with a timeout so we can provide a user friendly
/// error message if QGA never comes up in the guest.
pub struct QgaWrapper {
    send_req: Sender<QgaWrapperCommand>,
    recv_resp: Receiver<Result<QgaWrapperCommandResp>>,
}

#[allow(clippy::enum_variant_names)]
pub enum QgaWrapperCommand {
    GuestSync,
    GuestExec(qga::guest_exec),
    GuestExecStatus(qga::guest_exec_status),
}

#[allow(clippy::enum_variant_names)]
pub enum QgaWrapperCommandResp {
    GuestSync,
    GuestExec(<qga::guest_exec as QapiCommand>::Ok),
    GuestExecStatus(<qga::guest_exec_status as QapiCommand>::Ok),
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
        while Instant::now() < end {
            let qga_stream = match UnixStream::connect(&sock) {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to connect QGA, retrying: {}", e);
                    continue;
                }
            };
            qga_stream.set_read_timeout(Some(Duration::from_secs(1)))?;
            let mut qga = Qga::from_stream(&qga_stream);
            let sync_value = &qga_stream as *const _ as usize as i32;
            if let Ok(_) = qga.guest_sync(sync_value) {
                break;
            }
        }
        let (send_req, recv_req) = mpsc::channel();
        let (send_resp, recv_resp) = mpsc::channel();

        // Start worker thread to service requests
        thread::spawn(move || Self::worker(sock, recv_req, send_resp));

        let r = Self {
            send_req,
            recv_resp,
        };

        r.guest_sync(timeout)?;

        Ok(r)
    }

    /// Worker thread that fields QGA requests from the main thread
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

    /// Ask the worker thread to complete a request
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

    /// Sync with qemu-ga inside guest
    pub fn guest_sync(&self, timeout: Duration) -> Result<()> {
        self.execute(QgaWrapperCommand::GuestSync, timeout)
            .map(|_| ())
    }

    /// Run a command inside the guest
    pub fn guest_exec(
        &self,
        args: qga::guest_exec,
    ) -> Result<<qga::guest_exec as QapiCommand>::Ok> {
        match self.execute(QgaWrapperCommand::GuestExec(args), Duration::MAX)? {
            QgaWrapperCommandResp::GuestExec(resp) => Ok(resp),
            _ => panic!("Impossible return"),
        }
    }

    /// Query status of a command inside the guest
    pub fn guest_exec_status(
        &self,
        pid: i64,
    ) -> Result<<qga::guest_exec_status as QapiCommand>::Ok> {
        match self.execute(
            QgaWrapperCommand::GuestExecStatus(qga::guest_exec_status { pid }),
            Duration::from_secs(5),
        )? {
            QgaWrapperCommandResp::GuestExecStatus(resp) => Ok(resp),
            _ => panic!("Impossible return"),
        }
    }
}
