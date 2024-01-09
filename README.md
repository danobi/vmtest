# vmtest

[![CI](https://github.com/danobi/vmtest/actions/workflows/rust.yml/badge.svg)](https://github.com/danobi/vmtest/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/vmtest.svg)](https://crates.io/crates/vmtest)

`vmtest` enables you to quickly and programmatically run tests inside a virtual
machine.

This can be useful in the following, non-exhaustive, list of scenarios:

* You ship a virtual machine image and you want to programmatically test the
  image during development, both locally and in CI.
* You develop eBPF-powered applications and you want to run your application
  tests on a variety of kernels your application supports, both locally and in
  CI.
* You are a kernel developer and you want to quickly iterate on changes.

A key feature is that the root host userspace can-be/is transparently mapped
into the guest VM. This makes dropping `vmtest` into existing CI workflows
easy, as dependencies installed on the root host can also be effortlessly
reused inside the guest VM.

## Dependencies

The following are required dependencies, grouped by location:

Host machine:

* [`qemu`](https://pkgs.org/download/qemu)
* [`qemu-guest-agent`](https://pkgs.org/search/?q=qemu-guest-agent)
* [`OVMF`](https://pkgs.org/download/ovmf)

Virtual machine image:

* `qemu-guest-agent`
* Kernel 9p filesystem support, either compiled in or as modules (see kernel
  dependencies)
    * Most (if not all) distros already ship support as modules or better

Kernel:

* `CONFIG_VIRTIO=y`
* `CONFIG_VIRTIO_PCI=y`
* `CONFIG_VIRTIO_CONSOLE=y`
* `CONFIG_NET_9P=y`
* `CONFIG_NET_9P_VIRTIO=y`
* `CONFIG_9P_FS=y`

Note the virtual machine image dependencies are only required if you're using
the `image` target parameter. Likewise, the same applies for kernel
dependencies.

## Installation

Assuming you have a [`rust toolchain`](https://rustup.rs/) installed, simply
run:

```
$ cargo install vmtest
```

Alternatively, `vmtest` publishes statically linked binaries in its [release
assets](https://github.com/danobi/vmtest/releases). Currently only x86-64-linux
is published.

## Usage

### One-liner interface

The config file interface is more powerful and unlocks all `vmtest` features.
However it can be a bit heavyweight if you're just trying to do something
one-off. For such lighter-weight cases, `vmtest` has a one-liner interface.

For example, to run an arbitrary command in the guest VM with a different
kernel:

```
$ vmtest -k ./bzImage-v6.2 "uname -r"
=> bzImage-v6.2
===> Booting
===> Setting up VM
===> Running command
6.2.0
```

To run an arbitrary command in a guest VM with a different kernel and rootfs:
```
$ vmtest -k ./bzImage-v6.2 -r ./rootfs "uname -r"
=> bzImage-v6.2
===> Booting
===> Setting up VM
===> Running command
6.2.0
```

To run an arbitrary command from a kernel from another architecture in a guest VM:
```
$ vmtest -k ./kernels/Image-arm64 -r ./rootfs/ubuntu-lunar-arm64 -a aarch64 "uname -r"
=> Image-arm64
===> Booting
===> Setting up VM
===> Running command
6.6.0-rc5-ga4a0c99f10ca-dirty
```

See `vmtest --help` for all options and flags.

### Config file interface

`vmtest` by default reads from `vmtest.toml` in the current working directory.
`vmtest.toml`, in turn, describes which _targets_ should be run.

For example, consider the following `vmtest.toml`:

```
[[target]]
name = "AWS kernel"
kernel = "./bzImage-5.15.0-1022-aws"
command = "uname -r | grep -e aws$"

[[target]]
name = "OCI image"
image = "./oci-stage-6/oci-stage-6-disk001.qcow2"
command = "ls -l /mnt/vmtest && cat /proc/thiswillfail"

[[target]]
name = "Foreign Architecture"
kernel = "./kernels/Image-arm64"
arch = "aarch64"
rootfs = "./rootfs/ubuntu-lunar-arm64"
command = "uname -m | grep aarch64"
```

In the above config, two see two defined targets: "AWS kernel" and "OCI image".

In plain english, the "AWS kernel" target tells vmtest to run `command` in a VM
with the same userspace environment as the host, except with the specified
`kernel`.

"OCI image", on the other hand, tells vmtest to run `command` inside the
provided VM image. The image completely defines the environment `command` is
run in with the exception of `/mnt/vmtest`. `/mnt/vmtest` (as we will see
below) contains the full directory tree of the host machine rooted at the
directory containing `vmtest.toml`. This directory tree is shared - **not
copied** - with both readable and writable permissions.

Running vmtest with the above config yields the following results:

```
$ vmtest
=> AWS kernel
PASS
=> OCI image
===> Booting
===> Setting up VM
===> Running command
total 2057916
drwxr-xr-x 1 ubuntu ubuntu        200 Nov 14 20:41 avx-gateway-oci-stage-6
-rw-r--r-- 1 ubuntu ubuntu   11631520 Feb  1 00:33 bzImage-5.15.0-1022-aws
-rw-r--r-- 1 ubuntu ubuntu        359 Feb  4 01:41 vmtest.toml
cat: /proc/thiswillfail: No such file or directory
Command failed with exit code: 1
FAILED
=> Foreign Architecture
===> Booting
===> Setting up VM
===> Running command
aarch64
```

For full configuration documentation, see [config.md](./docs/config.md).

## Usage in Github CI

[vmtest-action](https://github.com/danobi/vmtest-action) is a convenient
wrapper around `vmtest` that is designed to run inside Github Actions. See
`vmtest-action` documentation for more details.

## Technical details

For general architecture notes, see [architecture.md](./docs/architecture.md).

## Acknowledgements

Many thanks to [`drgn`'s
vmtest](https://github.com/osandov/drgn/tree/main/vmtest) by Omar Sandoval and
Andy Lutomirski's most excellent [`virtme`](https://github.com/amluto/virtme)
for providing both ideas and technical exploration.
