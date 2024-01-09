use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::mpsc::channel;

use lazy_static::lazy_static;
use regex::Regex;
use tempfile::tempdir_in;
use test_log::test;

use vmtest::output::Output;
use vmtest::ui::Ui;
use vmtest::Mount;
use vmtest::{Config, Target, VMConfig};

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
                ..Default::default()
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(asset("image-not-uefi.raw")),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                ..Default::default()
            },
        ],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let ui = Ui::new(vmtest);
    let failed = ui.run(&*FILTER_ALL, false);
    assert_eq!(failed, 0);
}

// Expect that when we run multiple targets, we get the correct number of failures.
#[test]
fn test_run_multiple_return_number_failures() {
    let config = Config {
        target: vec![
            Target {
                name: "uefi image boots with uefi flag".to_string(),
                image: Some(asset("image-uefi.raw-efi")),
                uefi: true,
                command: "exit 1".to_string(),
                ..Default::default()
            },
            Target {
                name: "uefi image boots with uefi flag 2".to_string(),
                image: Some(asset("image-uefi.raw-efi")),
                uefi: true,
                command: "exit 1".to_string(),
                ..Default::default()
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(asset("image-not-uefi.raw")),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                ..Default::default()
            },
        ],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let ui = Ui::new(vmtest);
    let failed = ui.run(&*FILTER_ALL, false);
    assert_eq!(failed, 2);
}

// Expect that when we run a single target, we return the return code of the command.
#[test]
fn test_run_single_return_number_return_code() {
    let config = Config {
        target: vec![Target {
            name: "not uefi image boots without uefi flag".to_string(),
            image: Some(asset("image-not-uefi.raw")),
            uefi: false,
            command: "exit 12".to_string(),
            ..Default::default()
        }],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let ui = Ui::new(vmtest);
    let failed = ui.run(&*FILTER_ALL, false);
    assert_eq!(failed, 12);
}

// Expect that when we fail to start the vm, we return 69 (EX_UNAVAILABLE).
#[test]
fn test_vmtest_infra_error() {
    let config = Config {
        target: vec![Target {
            name: "not an actual image, should return EX_UNAVAILABLE".to_string(),
            image: Some(asset("not_an_actual_image")),
            uefi: false,
            command: "exit 12".to_string(),
            ..Default::default()
        }],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let ui = Ui::new(vmtest);
    let failed = ui.run(&*FILTER_ALL, false);
    assert_eq!(failed, 69);
}

// Expect we can run each target one by one, sucessfully
#[test]
fn test_run_one() {
    let uefi_image = create_new_image(asset("image-uefi.raw-efi"));
    let non_uefi_image = create_new_image(asset("image-not-uefi.raw"));

    let config = Config {
        target: vec![
            Target {
                name: "uefi image boots with uefi flag".to_string(),
                image: Some(uefi_image.as_pathbuf()),
                uefi: true,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                ..Default::default()
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(non_uefi_image.as_pathbuf()),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                ..Default::default()
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
    let uefi_image = create_new_image(asset("image-uefi.raw-efi"));
    let non_uefi_image = create_new_image(asset("image-not-uefi.raw"));

    let config = Config {
        target: vec![
            Target {
                name: "uefi image boots with uefi flag".to_string(),
                image: Some(uefi_image.as_pathbuf()),
                uefi: true,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                ..Default::default()
            },
            Target {
                name: "not uefi image boots without uefi flag".to_string(),
                image: Some(non_uefi_image.as_pathbuf()),
                uefi: false,
                command: "/mnt/vmtest/main.sh nixos".to_string(),
                ..Default::default()
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
    let uefi_image = create_new_image(asset("image-uefi.raw-efi"));

    let config = Config {
        target: vec![Target {
            name: "uefi image does not boot without uefi flag".to_string(),
            image: Some(uefi_image.as_pathbuf()),
            uefi: false,
            command: "echo unreachable".to_string(),
            ..Default::default()
        }],
    };
    let (vmtest, _dir) = setup(config, &["main.sh"]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_err!(recv, Output::BootEnd);
}

#[test]
fn test_command_runs_in_shell() {
    let config = Config {
        target: vec![Target {
            name: "command is run in shell".to_string(),
            kernel: Some(asset("bzImage-v5.15-empty")),
            // `$0` is a portable way of getting the name of the shell without relying
            // on env vars which may be propagated from the host into the guest.
            command: "if true; then echo -n $0 > /mnt/vmtest/result; fi".to_string(),
            ..Default::default()
        }],
    };
    let (vmtest, dir) = setup(config, &[]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_no_err!(recv);

    // Check that output file contains the shell
    let result_path = dir.path().join("result");
    let result = fs::read_to_string(result_path).expect("Failed to read result");
    assert_eq!(result, "bash");
}

// Tests that for kernel targets, environment variables from the host are propagated
// into the guest.
#[test]
fn test_kernel_target_env_var_propagation() {
    let config = Config {
        target: vec![Target {
            name: "host env vars are propagated into guest".to_string(),
            kernel: Some(asset("bzImage-v5.15-empty")),
            command: "echo -n $TEST_ENV_VAR > /mnt/vmtest/result".to_string(),
            ..Default::default()
        }],
    };

    // Set test env var
    env::set_var("TEST_ENV_VAR", "test value");

    let (vmtest, dir) = setup(config, &[]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_no_err!(recv);

    // Check that output file contains the shell
    let result_path = dir.path().join("result");
    let result = fs::read_to_string(result_path).expect("Failed to read result");
    assert_eq!(result, "test value");
}

// Tests that for kernel targets, current working directory is preserved in the guest
#[test]
fn test_kernel_target_cwd_preserved() {
    let config = Config {
        target: vec![Target {
            name: "host cwd preserved in guest".to_string(),
            kernel: Some(asset("bzImage-v5.15-empty")),
            command: "cat text_file.txt".to_string(),
            ..Default::default()
        }],
    };

    // Calculate source fixture directory and pass it in as the base path
    // to `Vmtest`. The base path is what controls the working directory.
    //
    // Note the base path is used for other stuff too like resolving relative
    // paths in the config, but since we are careful to use absolute paths
    // in the config we can decouple that behavior for this test.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fixtures = root.join("tests/fixtures");
    let vmtest = vmtest::Vmtest::new(fixtures, config).expect("Failed to construct vmtest");

    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_no_err!(recv);
}

#[test]
fn test_command_process_substitution() {
    let config = Config {
        target: vec![Target {
            name: "command can run process substitution".to_string(),
            kernel: Some(asset("bzImage-v5.15-empty")),
            // `$0` is a portable way of getting the name of the shell without relying
            // on env vars which may be propagated from the host into the guest.
            command: "cat <(echo -n $0) > /mnt/vmtest/result".to_string(),
            ..Default::default()
        }],
    };
    let (vmtest, dir) = setup(config, &[]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_no_err!(recv);

    // Check that output file contains the shell
    let result_path = dir.path().join("result");
    let result = fs::read_to_string(result_path).expect("Failed to read result");
    assert_eq!(result, "bash");
}

#[test]
fn test_qemu_error_shown() {
    let config = Config {
        target: vec![Target {
            name: "invalid kernel path".to_string(),
            kernel: Some(asset("doesn't exist")),
            command: "true".to_string(),
            ..Default::default()
        }],
    };
    let (vmtest, _dir) = setup(config, &[]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);

    let err = assert_get_err!(recv, Output::BootEnd);
    let msg = err.to_string();
    assert!(msg.contains("qemu: could not open kernel file"));
}

// Test that host FS cannot be written to if `ro` flag is passed to guest kernel args
#[test]
fn test_kernel_ro_flag() {
    // Cannot place this dir in tmpfs b/c vmtest will mount over host /tmp with a new tmpfs
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let touch_dir = tempdir_in(root).expect("Failed to create tempdir");

    let config = Config {
        target: vec![Target {
            name: "cannot touch host rootfs with ro".to_string(),
            kernel: Some(asset("bzImage-v5.15-empty")),
            kernel_args: Some("ro".to_string()),
            command: format!("touch {}/file", touch_dir.path().display()),
            ..Default::default()
        }],
    };
    let (vmtest, _dir) = setup(config, &[]);
    let (send, recv) = channel();
    vmtest.run_one(0, send);
    assert_err!(recv, Output::CommandEnd, i64);
}

#[test]
fn test_run_custom_resources() {
    let uefi_image_t1 = create_new_image(asset("image-uefi.raw-efi"));
    let uefi_image_t2 = create_new_image(asset("image-uefi.raw-efi"));

    let config = Config {
        target: vec![
            Target {
                name: "Custom number of CPUs".to_string(),
                image: Some(uefi_image_t1.as_pathbuf()),
                uefi: true,
                command: r#"bash -xc "[[ "$(nproc)" == "1" ]]""#.into(),
                vm: VMConfig {
                    num_cpus: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
            Target {
                name: "Custom amount of RAM".to_string(),
                image: Some(uefi_image_t2.as_pathbuf()),
                uefi: true,
                // Should be in the 200 thousands, but it's variable.
                command: r#"bash -xc "cat /proc/meminfo | grep 'MemTotal:         2..... kB'""#
                    .into(),
                vm: VMConfig {
                    memory: "256M".into(),
                    ..Default::default()
                },
                ..Default::default()
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

#[test]
fn test_run_custom_mounts() {
    let uefi_image = create_new_image(asset("image-uefi.raw-efi"));

    let config = Config {
        target: vec![
            Target {
                name: "mount".to_string(),
                image: Some(uefi_image.as_pathbuf()),
                uefi: true,
                command: r#"bash -xc "[[ -e /tmp/mount/README.md ]]""#.into(),
                vm: VMConfig {
                    mounts: HashMap::from([(
                        "/tmp/mount".into(),
                        Mount {
                            host_path: Path::new(env!("CARGO_MANIFEST_DIR")).into(),
                            writable: true,
                        },
                    )]),
                    ..Default::default()
                },
                ..Default::default()
            },
            Target {
                name: "RO mount".to_string(),
                image: Some(uefi_image.as_pathbuf()),
                uefi: true,
                command: r#"bash -xc "(touch /tmp/ro/hi && exit -1) || true""#.into(),
                vm: VMConfig {
                    mounts: HashMap::from([(
                        "/tmp/ro".into(),
                        Mount {
                            host_path: Path::new(env!("CARGO_MANIFEST_DIR")).into(),
                            writable: false,
                        },
                    )]),
                    ..Default::default()
                },
                ..Default::default()
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
