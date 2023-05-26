use std::sync::mpsc::channel;

use lazy_static::lazy_static;
use regex::Regex;
use test_log::test;

use vmtest::output::Output;
use vmtest::ui::Ui;
use vmtest::{Config, Target};

mod helpers;
use helpers::*;

lazy_static! {
    static ref FILTER_ALL: Regex = Regex::new(".*").unwrap();
}

// Expect that we can run the entire matrix successfully
#[test]
fn test_run() {
    let config = Config {
        target: vec![
            Target {
                name: "uefi image boots with uefi flag".to_string(),
                image: Some(asset("image-uefi.raw-efi")),
                uefi: true,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                kernel: None,
                kernel_args: None,
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(asset("image-not-uefi.raw")),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                kernel: None,
                kernel_args: None,
            },
        ],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let ui = Ui::new(vmtest);
    let failed = ui.run(&*FILTER_ALL, false);
    assert_eq!(failed, 0);
}

// Expect we can run each target one by one, sucessfully
#[test]
fn test_run_one() {
    let config = Config {
        target: vec![
            Target {
                name: "uefi image boots with uefi flag".to_string(),
                image: Some(asset("image-uefi.raw-efi")),
                uefi: true,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                kernel: None,
                kernel_args: None,
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(asset("image-not-uefi.raw")),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                kernel: None,
                kernel_args: None,
            },
        ],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    for i in 0..2 {
        let (send, recv) = channel();
        vmtest.run_one(i, send);
        assert_no_err!(recv);
    }
}

// Expect that we have bounds checks
#[test]
fn test_run_out_of_bounds() {
    let config = Config {
        target: vec![
            Target {
                name: "uefi image boots with uefi flag".to_string(),
                image: Some(asset("image-uefi.raw-efi")),
                uefi: true,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                kernel: None,
                kernel_args: None,
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(asset("image-not-uefi.raw")),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                kernel: None,
                kernel_args: None,
            },
        ],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let (send, recv) = channel();
    vmtest.run_one(2, send);
    assert_err!(recv, Output::BootEnd);
}

// Try running a uefi image without uefi flag. It should fail to boot.
#[test]
fn test_not_uefi() {
    let config = Config {
        target: vec![Target {
            name: "uefi image does not boot without uefi flag".to_string(),
            image: Some(asset("image-uefi.raw-efi")),
            uefi: false,
            command: "echo unreachable".to_string(),
            kernel: None,
            kernel_args: None,
        }],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_err!(recv, Output::BootEnd);
}
