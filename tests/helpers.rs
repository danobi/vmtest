use std::env;
use std::fs;
use std::mem::{discriminant, Discriminant};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
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

fn found_error(recv: Receiver<Output>, disc: Option<Discriminant<Output>>) -> bool {
    let mut found_err = false;

    loop {
        let msg = match recv.recv() {
            Ok(m) => m,
            // Hangup means the end
            Err(_) => break,
        };

        match msg {
            Output::BootEnd(Err(_)) | Output::SetupEnd(Err(_)) | Output::CommandEnd(Err(_)) => {
                if let Some(d) = disc {
                    if discriminant(&msg) == d {
                        found_err = true;
                    }
                } else {
                    found_err = true;
                }
            }
            Output::CommandEnd(Ok(rc)) => {
                if let Some(d) = disc {
                    if discriminant(&msg) == d && rc != 0 {
                        found_err = true;
                    }
                } else if rc != 0 {
                    found_err = true;
                }
            }
            _ => (),
        };
    }

    found_err
}

// Assert that an error has been received
pub fn assert_error(recv: Receiver<Output>, disc: Discriminant<Output>) {
    assert!(found_error(recv, Some(disc)));
}

// Assert that no errors have been received
pub fn assert_no_error(recv: Receiver<Output>) {
    assert!(!found_error(recv, None));
}
