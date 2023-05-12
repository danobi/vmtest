use std::env;
use std::fs;
use std::mem::{discriminant, Discriminant};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};

use lazy_static::lazy_static;
use regex::Regex;
use test_log::test;

use vmtest::output::Output;
use vmtest::ui::Ui;
use vmtest::vmtest::Vmtest;

lazy_static! {
    static ref FILTER_ALL: Regex = Regex::new(".*").unwrap();
}

// Change working directory into integration test dir
fn chdir() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let testdir = root.join("tests/");
    env::set_current_dir(&testdir).expect("Failed to set testdir");

    testdir
}

// Create `Vmtest` instance from `vmtest.toml` in cwd
fn vmtest(filename: &str) -> Vmtest {
    let testdir = chdir();
    let contents = fs::read_to_string(filename).expect("Failed to read config");
    let config = toml::from_str(&contents).expect("Failed to parse config");
    let vmtest = vmtest::Vmtest::new(testdir, config).expect("Failed to construct vmtest");

    vmtest
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
fn assert_error(recv: Receiver<Output>, disc: Discriminant<Output>) {
    assert!(found_error(recv, Some(disc)));
}

// Assert that no errors hav been received
fn assert_no_error(recv: Receiver<Output>) {
    assert!(!found_error(recv, None));
}

// Expect that we can run the entire matrix successfully
#[test]
fn test_run() {
    let vmtest = vmtest("vmtest.toml.allgood");
    let ui = Ui::new(vmtest);
    let failed = ui.run(&*FILTER_ALL, false);
    assert_eq!(failed, 0);
}

// Expect we can run each target one by one, sucessfully
#[test]
fn test_run_one() {
    let vmtest = vmtest("vmtest.toml.allgood");
    for i in 0..2 {
        let (send, recv) = channel();
        vmtest.run_one(i, send);
        assert_no_error(recv);
    }
}

// Expect that we have bounds checks
#[test]
fn test_run_out_of_bounds() {
    let vmtest = vmtest("vmtest.toml.allgood");
    let (send, recv) = channel();
    vmtest.run_one(2, send);
    assert_error(recv, discriminant(&Output::BootEnd(Ok(()))));
}

// The mkosi images only support UEFI boot. Expect that by not specifying
// `uefi = true` in config, target fails
#[test]
fn test_not_uefi() {
    let vmtest = vmtest("vmtest.toml.notuefi");
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_error(recv, discriminant(&Output::BootEnd(Ok(()))));

    let (send, recv) = channel();
    vmtest.run_one(1, send);
    assert_no_error(recv);
}
