use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::RwLock;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use anyhow::bail;
use anyhow::Context as _;
use anyhow::Error;
use anyhow::Result;

use log::error;
use log::warn;

use vhost::vhost_user::Backend;
use vhost::vhost_user::VhostUserProtocolFeatures;
use vhost::vhost_user::VhostUserVirtioFeatures;
use vhost_user_backend::bitmap::BitmapMmapRegion;
use vhost_user_backend::VhostUserBackend;
use vhost_user_backend::VhostUserDaemon;
use vhost_user_backend::VringMutex;
use vhost_user_backend::VringState;
use vhost_user_backend::VringT;

use virtio_bindings::virtio_config::VIRTIO_F_VERSION_1;
use virtio_bindings::virtio_ring::VIRTIO_RING_F_EVENT_IDX;
use virtio_bindings::virtio_ring::VIRTIO_RING_F_INDIRECT_DESC;
use virtio_queue::QueueOwnedT;
use virtiofsd::descriptor_utils::Reader;
use virtiofsd::descriptor_utils::Writer;
use virtiofsd::filesystem::{FileSystem, SerializableFileSystem};
use virtiofsd::passthrough;
use virtiofsd::passthrough::CachePolicy;
use virtiofsd::passthrough::PassthroughFs;
use virtiofsd::server::Server;

use vm_memory::GuestAddressSpace;
use vm_memory::GuestMemoryAtomic;
use vm_memory::GuestMemoryMmap;

use vmm_sys_util::epoll::EventSet;
use vmm_sys_util::eventfd::EventFd;

use crate::util::gen_sock;

type LoggedMemory = GuestMemoryMmap<BitmapMmapRegion>;
type LoggedMemoryAtomic = GuestMemoryAtomic<LoggedMemory>;

const QUEUE_SIZE: usize = 32768;
// The spec allows for multiple request queues. We currently only support one.
const REQUEST_QUEUES: u32 = 1;
// In addition to the request queue there is one high-prio queue.
// Since VIRTIO_FS_F_NOTIFICATION is not advertised we do not have a
// notification queue.
const NUM_QUEUES: usize = REQUEST_QUEUES as usize + 1;
// The guest queued an available buffer for the high priority queue.
const HIPRIO_QUEUE_EVENT: u16 = 0;
// The guest queued an available buffer for the request queue.
const REQ_QUEUE_EVENT: u16 = 1;

struct VhostUserFsThread<F: FileSystem + Send + Sync + 'static> {
    mem: Option<LoggedMemoryAtomic>,
    kill_evt: EventFd,
    server: Arc<Server<F>>,
    // handle request from backend to frontend
    vu_req: Option<Backend>,
    event_idx: bool,
}

impl<F: FileSystem + SerializableFileSystem + Send + Sync + 'static> VhostUserFsThread<F> {
    fn new(fs: F) -> Result<Self> {
        Ok(VhostUserFsThread {
            mem: None,
            kill_evt: EventFd::new(libc::EFD_NONBLOCK).context("failed to create eventfd")?,
            server: Arc::new(Server::new(fs)),
            vu_req: None,
            event_idx: false,
        })
    }

    fn return_descriptor(
        vring_state: &mut VringState<LoggedMemoryAtomic>,
        head_index: u16,
        event_idx: bool,
        len: usize,
    ) {
        let used_len: u32 = match len.try_into() {
            Ok(l) => l,
            Err(_) => panic!("Invalid used length, can't return used descriptors to the ring"),
        };

        if vring_state.add_used(head_index, used_len).is_err() {
            warn!("couldn't return used descriptors to the ring");
        }

        if event_idx {
            match vring_state.needs_notification() {
                Err(_) => {
                    warn!("couldn't check if queue needs to be notified");
                    vring_state.signal_used_queue().unwrap();
                }
                Ok(needs_notification) => {
                    if needs_notification {
                        vring_state.signal_used_queue().unwrap();
                    }
                }
            }
        } else {
            vring_state.signal_used_queue().unwrap();
        }
    }

    fn process_queue_serial(
        &self,
        vring_state: &mut VringState<LoggedMemoryAtomic>,
    ) -> io::Result<bool> {
        let mut used_any = false;
        let mem = match self.mem.as_ref() {
            Some(m) => m.memory(),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "no memory configuration present",
                ))
            }
        };

        let mut vu_req = self.vu_req.clone();

        let avail_chains = vring_state
            .get_queue_mut()
            .iter(mem.clone())
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?
            .collect::<Vec<_>>();

        for chain in avail_chains {
            used_any = true;

            let head_index = chain.head_index();

            let reader = Reader::new(&mem, chain.clone()).unwrap();
            let writer = Writer::new(&mem, chain.clone()).unwrap();

            let len = self
                .server
                .handle_message(reader, writer, vu_req.as_mut())
                .unwrap();

            Self::return_descriptor(vring_state, head_index, self.event_idx, len);
        }

        Ok(used_any)
    }

    fn handle_event_serial(
        &self,
        device_event: u16,
        vrings: &[VringMutex<LoggedMemoryAtomic>],
    ) -> io::Result<()> {
        let mut vring_state = match device_event {
            HIPRIO_QUEUE_EVENT => vrings[0].get_mut(),
            REQ_QUEUE_EVENT => vrings[1].get_mut(),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("received unknown device event: {device_event}"),
                ))
            }
        };

        if self.event_idx {
            // vm-virtio's Queue implementation only checks avail_index
            // once, so to properly support EVENT_IDX we need to keep
            // calling process_queue() until it stops finding new
            // requests on the queue.
            loop {
                vring_state.disable_notification().unwrap();
                // we can't recover from an error here, so let's hope it's transient
                if let Err(e) = self.process_queue_serial(&mut vring_state) {
                    error!("processing the vring: {e}");
                }
                if !vring_state.enable_notification().unwrap() {
                    break;
                }
            }
        } else {
            // Without EVENT_IDX, a single call is enough.
            self.process_queue_serial(&mut vring_state)?;
        }

        Ok(())
    }
}

struct PremigrationThread {
    handle: JoinHandle<io::Result<()>>,
    cancel: Arc<AtomicBool>,
}

struct VhostUserFsBackend<F: FileSystem + SerializableFileSystem + Send + Sync + 'static> {
    thread: RwLock<VhostUserFsThread<F>>,
    premigration_thread: Mutex<Option<PremigrationThread>>,
    migration_thread: Mutex<Option<JoinHandle<io::Result<()>>>>,
}

impl<F: FileSystem + SerializableFileSystem + Send + Sync + 'static> VhostUserFsBackend<F> {
    fn new(fs: F) -> Result<Self> {
        let thread = RwLock::new(VhostUserFsThread::new(fs)?);
        Ok(VhostUserFsBackend {
            thread,
            premigration_thread: None.into(),
            migration_thread: None.into(),
        })
    }
}

impl<F: FileSystem + SerializableFileSystem + Send + Sync + 'static> VhostUserBackend
    for VhostUserFsBackend<F>
{
    type Bitmap = BitmapMmapRegion;
    type Vring = VringMutex<LoggedMemoryAtomic>;

    fn num_queues(&self) -> usize {
        NUM_QUEUES
    }

    fn max_queue_size(&self) -> usize {
        QUEUE_SIZE
    }

    fn features(&self) -> u64 {
        1 << VIRTIO_F_VERSION_1
            | 1 << VIRTIO_RING_F_INDIRECT_DESC
            | 1 << VIRTIO_RING_F_EVENT_IDX
            | VhostUserVirtioFeatures::PROTOCOL_FEATURES.bits()
            | VhostUserVirtioFeatures::LOG_ALL.bits()
    }

    fn protocol_features(&self) -> VhostUserProtocolFeatures {
        VhostUserProtocolFeatures::MQ
            | VhostUserProtocolFeatures::BACKEND_REQ
            | VhostUserProtocolFeatures::BACKEND_SEND_FD
            | VhostUserProtocolFeatures::REPLY_ACK
            | VhostUserProtocolFeatures::CONFIGURE_MEM_SLOTS
            | VhostUserProtocolFeatures::LOG_SHMFD
            | VhostUserProtocolFeatures::DEVICE_STATE
    }

    fn acked_features(&self, features: u64) {
        if features & VhostUserVirtioFeatures::LOG_ALL.bits() != 0 {
            // F_LOG_ALL set: Prepare for migration (unless we're already doing that)
            let mut premigration_thread = self.premigration_thread.lock().unwrap();
            if premigration_thread.is_none() {
                let cancel = Arc::new(AtomicBool::new(false));
                let cloned_server = Arc::clone(&self.thread.read().unwrap().server);
                let cloned_cancel = Arc::clone(&cancel);
                let handle =
                    thread::spawn(move || cloned_server.prepare_serialization(cloned_cancel));
                *premigration_thread = Some(PremigrationThread { handle, cancel });
            }
        } else {
            // F_LOG_ALL cleared: Migration cancelled, if any was ongoing
            // (Note that this is our interpretation, and not said by the specification.  The back
            // end might clear this flag also on the source side once the VM has been stopped, even
            // before we receive SET_DEVICE_STATE_FD.  QEMU will clear F_LOG_ALL only when the VM
            // is running, i.e. when the source resumes after a cancelled migration, which is
            // exactly what we want, but it would be better if we had a more reliable way that is
            // backed up by the spec.  We could delay cancelling until we receive a guest request
            // while F_LOG_ALL is cleared, but that can take an indefinite amount of time.)
            if let Some(premigration_thread) = self.premigration_thread.lock().unwrap().take() {
                premigration_thread.cancel.store(true, Ordering::Relaxed);
                // Ignore the result, we are cancelling anyway
                let _ = premigration_thread.handle.join();
            }
        }
    }

    fn set_event_idx(&self, enabled: bool) {
        self.thread.write().unwrap().event_idx = enabled;
    }

    fn update_memory(&self, mem: LoggedMemoryAtomic) -> io::Result<()> {
        self.thread.write().unwrap().mem = Some(mem);
        Ok(())
    }

    fn handle_event(
        &self,
        device_event: u16,
        evset: EventSet,
        vrings: &[VringMutex<LoggedMemoryAtomic>],
        _thread_id: usize,
    ) -> io::Result<()> {
        if evset != EventSet::IN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid event set",
            ));
        }

        let thread = self.thread.read().unwrap();
        thread.handle_event_serial(device_event, vrings)
    }

    fn check_device_state(&self) -> io::Result<()> {
        // Our caller (vhost-user-backend crate) pretty much ignores error objects we return (only
        // cares whether we succeed or not), so log errors here
        if let Err(err) = self.do_check_device_state() {
            error!("Failed to conclude migration: {err}");
            return Err(err);
        }
        Ok(())
    }
}

impl<F: FileSystem + SerializableFileSystem + Send + Sync + 'static> VhostUserFsBackend<F> {
    fn do_check_device_state(&self) -> io::Result<()> {
        let result = if let Some(migration_thread) = self.migration_thread.lock().unwrap().take() {
            // `Result::flatten()` is not stable yet, so no `.join().map_err(...).flatten()`
            match migration_thread.join() {
                Ok(x) => x,
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::Other,
                    "Failed to join the migration thread",
                )),
            }
        } else {
            // `check_device_state()` must follow a successful `set_device_state_fd()`, so this is
            // a protocol violation
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Front-end attempts to check migration state, but no migration has been done",
            ))
        };

        // Note that just like any other vhost-user message implementation, the error object that
        // we return is not forwarded to the front end (it only receives an error flag), so if we
        // want users to see some diagnostics, we have to print them ourselves
        if let Err(e) = &result {
            error!("Migration failed: {e}");
        }
        result
    }
}

enum Either<A, B> {
    A(A),
    B(B),
}

pub(crate) struct Virtiofsd {
    fs_backend: Arc<VhostUserFsBackend<PassthroughFs>>,
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
            fs_backend.clone(),
            GuestMemoryAtomic::new(GuestMemoryMmap::new()),
        )
        .map_err(|err| Error::msg(err.to_string()))
        .context("failed to instantiate vhost user daemon")?;

        let slf = Self {
            fs_backend,
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

impl Drop for Virtiofsd {
    fn drop(&mut self) {
        // Ideally we'd await the server thread, but that can
        // conceptually block for a long time and shouldn't be done
        // inside a constructor.

        let kill_evt = self
            .fs_backend
            .thread
            .read()
            .unwrap()
            .kill_evt
            .try_clone()
            .unwrap();
        if let Err(err) = kill_evt.write(1) {
            error!("failed to shut down worker thread: {err:#}");
        }
    }
}
