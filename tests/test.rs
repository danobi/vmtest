use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use test_log::test;

use vmtest::vmtest::Vmtest;

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

// Expect that we can run the entire matrix successfully
#[test]
fn test_run() {
    let vmtest = vmtest("vmtest.toml.allgood");
    vmtest.run().expect("Failed to vmtest.run()");
}

// Expect we can run each target one by one, sucessfully
#[test]
fn test_run_one() {
    let vmtest = vmtest("vmtest.toml.allgood");
    for i in 0..2 {
        let result = vmtest.run_one(i).expect("Failed to vmtest.run_one()");
        assert_eq!(result.exitcode, 0);
    }

    vmtest.run_one(2).expect_err("Should only have 2 targets");
}

// The mkosi images only support UEFI boot. Expect that by not specifying
// `uefi = true` in config, target fails
#[test]
fn test_not_uefi() {
    let vmtest = vmtest("vmtest.toml.notuefi");

    vmtest
        .run_one(0)
        .expect_err("Not uefi image should have failed");

    let result = vmtest.run_one(1).expect("uefi should succeed");
    assert_eq!(result.exitcode, 0);
}
