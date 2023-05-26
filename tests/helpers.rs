use std::env;
use std::fs;
use std::mem::{discriminant, Discriminant};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use anyhow::{anyhow, Error};
use tempdir::TempDir;

use vmtest::output::Output;
use vmtest::vmtest::Vmtest;
use vmtest::Config;

// Returns a path to a test asset
pub fn asset(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let asset = root.join("tests/.assets").join(name);
    asset
}

// Set up a test run
//
// This will create a tempdir, chdir to it, lay down any requested fixtures,
// and initialize a `Vmtest` instance given the config.
//
// Note: tests must hold onto the tempdir handle until the test is over.
pub fn setup(config: Config, fixtures: &[&str]) -> (Vmtest, TempDir) {
    let dir = TempDir::new("vmtest-test").expect("Failed to create tempdir");
    env::set_current_dir(dir.path()).expect("Failed to set testdir");

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

#[macro_export]
macro_rules! assert_err {
    ($recv:expr, $variant:path) => {
        use std::mem::discriminant;

        // The `Ok(())` is not used at all. We just need something to initialize
        // the enum with b/c `discriminant()` takes values, not identifiers.
        let d = discriminant(&$variant(Ok(())));
        assert!(get_error($recv, Some(d)).is_some());
    };
}

#[macro_export]
macro_rules! assert_no_err {
    ($recv:expr) => {
        assert!(get_error($recv, None).is_none());
    };
}
