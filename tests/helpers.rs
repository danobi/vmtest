use std::env;
use std::fs;
use std::mem::{discriminant, Discriminant};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Receiver;

use anyhow::{anyhow, Error};
use rand::Rng;
use tempfile::{tempdir, TempDir};

use vmtest::output::Output;
use vmtest::vmtest::Vmtest;
use vmtest::Config;

// Returns a path to a test asset
pub fn asset(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));

    root.join("tests/.assets").join(name)
}

// Set up a test run
//
// This will create a tempdir, chdir to it, lay down any requested fixtures,
// and initialize a `Vmtest` instance given the config.
//
// Note: tests must hold onto the tempdir handle until the test is over.
pub fn setup(config: Config, fixtures: &[&str]) -> (Vmtest, TempDir) {
    let dir = tempdir().expect("Failed to create tempdir");

    for fixture in fixtures {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let file = root.join("tests/fixtures").join(fixture);
        fs::copy(file, dir.path().join(fixture)).expect("Failed to copy fixture");
    }

    let vmtest = vmtest::Vmtest::new(dir.path(), config).expect("Failed to construct vmtest");
    (vmtest, dir)
}

// Should not be called outside of this file
#[doc(hidden)]
pub fn get_error(recv: Receiver<Output>, disc: Option<Discriminant<Output>>) -> Option<Error> {
    loop {
        let msg = match recv.recv() {
            Ok(m) => m,
            // Hangup means the end
            Err(_) => break,
        };

        let msg_disc = discriminant(&msg);

        match msg {
            Output::BootEnd(Err(e)) | Output::SetupEnd(Err(e)) | Output::CommandEnd(Err(e)) => {
                if let Some(d) = disc {
                    if msg_disc == d {
                        return Some(e);
                    }
                } else {
                    return Some(e);
                }
            }
            // Helpful for debugging test failures
            Output::Command(s) => println!("Command output={s}"),
            Output::CommandEnd(Ok(rc)) => {
                if let Some(d) = disc {
                    if msg_disc == d && rc != 0 {
                        return Some(anyhow!("Command failed with {}", rc));
                    }
                } else if rc != 0 {
                    return Some(anyhow!("Command failed with {}", rc));
                }
            }
            _ => (),
        };
    }

    None
}

/// Macro to help assert that an error has been received.
///
/// Example usage:
///
/// ```ignore
/// assert_err!(recv, Output::BootEnd);
/// assert_err!(recv, Output::CommandEnd, i64);
/// ```
#[macro_export]
macro_rules! assert_err {
    ($recv:expr, $variant:path) => {
        use std::mem::discriminant;

        // The `Ok(())` is not used at all. We just need something to initialize
        // the enum with b/c `discriminant()` takes values, not identifiers.
        let d = discriminant(&$variant(Ok(())));
        assert!(dbg!(get_error($recv, Some(d))).is_some());
    };

    ($recv:expr, $variant:path, $ty:ty) => {
        use std::mem::discriminant;

        // Just like above, the default value does not matter.
        let d = discriminant(&$variant(Ok(<$ty>::default())));
        assert!(dbg!(get_error($recv, Some(d))).is_some());
    };
}

#[macro_export]
macro_rules! assert_get_err {
    ($recv:expr, $variant:path) => {{
        use std::mem::discriminant;

        // The `Ok(())` is not used at all. We just need something to initialize
        // the enum with b/c `discriminant()` takes values, not identifiers.
        let d = discriminant(&$variant(Ok(())));
        get_error($recv, Some(d)).expect("Failed to find error")
    }};
}

#[macro_export]
macro_rules! assert_no_err {
    ($recv:expr) => {
        assert!(dbg!(get_error($recv, None)).is_none());
    };
}

fn gen_image_name() -> PathBuf {
    let mut path = PathBuf::new();
    path.push("/tmp");

    let id = rand::thread_rng().gen_range(100_000..1_000_000);
    let image = format!("/tmp/image-{id}.qcow2");
    path.push(image);

    path
}

/// Wrapper around a pathbuf that deletes the file at drop
pub struct CowImage {
    path: PathBuf,
}

impl CowImage {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn as_pathbuf(&self) -> PathBuf {
        self.path.clone()
    }
}

impl Drop for CowImage {
    fn drop(&mut self) {
        fs::remove_file(&self.path).expect("Failed to remove CoW'd image");
    }
}

// Create a CoW image to ensure each test runs in a clean image.
//
// We've seen some really odd flakiness issues when the same image
// is reused. It may be a nixos thing. Still unclear.
pub fn create_new_image(image: PathBuf) -> CowImage {
    let out_image = gen_image_name();
    let out = Command::new("qemu-img")
        .arg("create")
        .arg("-F")
        .arg("raw")
        .arg("-b")
        .arg(image)
        .arg("-f")
        .arg("qcow2")
        .arg(out_image.clone())
        .output()
        .expect("error creating image file");
    if !out.status.success() {
        panic!(
            "error creating image file: out={} err={} status={}",
            std::str::from_utf8(&out.stdout).unwrap(),
            std::str::from_utf8(&out.stderr).unwrap(),
            out.status
        );
    }

    CowImage::new(out_image)
}
