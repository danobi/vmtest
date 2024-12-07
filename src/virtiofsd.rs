use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use vhost_user_backend::VhostUserDaemon;

use virtiofsd::passthrough;
use virtiofsd::passthrough::CachePolicy;
use virtiofsd::passthrough::PassthroughFs;
use virtiofsd::vhost_user::VhostUserFsBackend;
use vm_memory::GuestMemoryAtomic;
use vm_memory::GuestMemoryMmap;

use crate::util::gen_sock;

enum Either<A, B> {
    A(A),
    B(B),
}

pub(crate) struct Virtiofsd {
    #[allow(clippy::type_complexity)]
    state: Either<
        Option<VhostUserDaemon<Arc<VhostUserFsBackend<PassthroughFs>>>>,
        JoinHandle<Result<(), vhost_user_backend::Error>>,
    >,
    /// The path to the Unix domain socket used for communication.
    socket_path: PathBuf,
}

impl Virtiofsd {
    /// Create a `Virtiofsd` instance for sharing the given directory.
    pub fn new(shared_dir: &Path) -> Result<Self> {
        let socket = gen_sock("virtiofsd");
        let cache_policy = CachePolicy::Always;
        let timeout = match cache_policy {
            CachePolicy::Never => Duration::from_secs(0),
            CachePolicy::Metadata => Duration::from_secs(86400),
            CachePolicy::Auto => Duration::from_secs(1),
            CachePolicy::Always => Duration::from_secs(86400),
        };

        let fs_cfg = passthrough::Config {
            entry_timeout: timeout,
            attr_timeout: timeout,
            cache_policy,
            root_dir: shared_dir
                .to_str()
                .context("shared directory is not a valid UTF-8 string")?
                .to_string(),
            announce_submounts: true,
            ..Default::default()
        };

        let fs = PassthroughFs::new(fs_cfg)
            .context("failed to create internal filesystem representation")?;
        let fs_backend =
            Arc::new(VhostUserFsBackend::new(fs).context("error creating vhost-user backend")?);

        let daemon = VhostUserDaemon::new(
            String::from("virtiofsd-backend"),
            fs_backend,
            GuestMemoryAtomic::new(GuestMemoryMmap::new()),
        )
        .map_err(|err| Error::msg(err.to_string()))
        .context("failed to instantiate vhost user daemon")?;

        let slf = Self {
            state: Either::A(Some(daemon)),
            socket_path: socket,
        };
        Ok(slf)
    }

    pub fn launch(&mut self) -> Result<()> {
        if let Either::A(ref mut daemon) = &mut self.state {
            let mut daemon = daemon.take().unwrap();
            let socket = self.socket_path.clone();
            self.state = Either::B(thread::spawn(move || daemon.serve(socket)));
        }
        Ok(())
    }

    pub fn await_launched(&mut self) -> Result<()> {
        if let Either::A(..) = self.state {
            let () = self.launch()?;
        }

        match self.state {
            Either::A(..) => unreachable!(),
            Either::B(..) => {
                let now = Instant::now();
                let timeout = Duration::from_secs(5);

                while now.elapsed() < timeout {
                    if self.socket_path.exists() {
                        return Ok(());
                    }

                    thread::sleep(Duration::from_millis(1));
                }
            }
        };

        bail!(
            "virtiofsd socket `{}` did not appear in time",
            self.socket_path.display()
        )
    }

    #[inline]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }
}
