# vmtest

**Currently under active development. Feedback is welcome.**

`vmtest` enables you to quickly and programmatically run tests inside a virtual
machine.

This can be useful in the following, non-exhaustive, list of scenarios:

* You ship a virtual machine image and you want to programmatically test the
  image during development, both locally and in CI.
* You develop eBPF-powered applications and you want to run your application
  tests on a variety of kernels your application supports, both locally and in
  CI.
* You are a kernel developer and you want to quickly iterate on changes.

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
* `CONFIG_NET_9P=y`
* `CONFIG_NET_9P_VIRTIO=y`
* `CONFIG_9P_FS=y`

Note the virtual machine image dependencies are only required if you're using
the `image` target parameter. Likewise, the same applies for kernel
dependencies.

## Usage

`vmtest` by default reads from `vmtest.toml` in the current working directory.
`vmtest.toml`, in turn, describes which _targets_ should be run.

For example, consider the following `vmtest.toml`:

```
[[target]]
name = "AWS kernel"
kernel = "./bzImage-5.15.0-1022-aws
command = "/bin/bash -c 'uname -r | grep -e aws$'"

[[target]]
name = "OCI image"
image = "./oci-stage-6/oci-stage-6-disk001.qcow2"
command = "/bin/bash -c 'ls -l /mnt/vmtest && cat /proc/thiswillfail'"
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
```

## Configuration

The following sections are supported:

### `[[target]]`

Each target is specified using a `[[target]]` section. In TOML, this is known
as an [array of tables](https://toml.io/en/v1.0.0-rc.3#array-of-tables).

The following fields are supported:

* `name` (string)
    * Required field
    * The name of the target. The name is used for documentation and
      identification purposes.
* `image` (string)
    * Optional field, but one of `image` and `kernel` must be specified
    * The path to the virtual machine image
    * If a relative path is provided, it will be interpreted as relative to
      `vmtest.toml`
* `uefi` (boolean)
    * Default: `false`
    * Whether to use UEFI boot or not
    * `false` implies BIOS boot
* `kernel` (string)
    * Optional field, but one of `image` and `kernel` must be specified
    * The path to the kernel to use
    * Typically named `vmlinuz` or `bzImage`
    * If a relative path is provided, it will be interpreted as relative to
      `vmtest.toml`
* `command` (string)
    * Required field
    * Command to run inside VM
    * The specified command must be an absolute path
    * Note that the specified command is not run inside a shell by default.
      If you want a shell, use `/bin/bash -c "$SHELL_CMD_HERE"`.


## Technical details

### Github actions

`vmtest` is designed to be useful for both local development and running tests
in CI. As part of vmtest development, we run integration tests inside github
actions. This means you can be sure that vmtest will run in github actions
straight out of the box.

See [the integration tests
here](https://github.com/danobi/vmtest/blob/master/.github/workflows/rust.yml).

Note that b/c smaller azure machine sizes (the ones github uses) don't support
nested virtualizaion, vmtest currently does full emulation inside GHA.

### Architecture

![General architecture](./docs/architecture.png)

The first big idea is that `vmtest` tries to orchestrate everything through
QEMU's programmable interfaces, namely the QEMU machine protocol (QMP) for
orchestrating QEMU and qemu-guest-agent (which also uses QMP under the hood)
for running things inside the guest VM. Both interfaces use a unix domain
socket for transport.

For image targets, we require that `qemu-guest-agent` is installed inside the
image b/c it's typically configured to auto-start through udev when the
appropriate virtio-serial device appears. This gives vmtest a clean out-of-band
mechanism to execute commands inside the guest. For kernel targets, we require
qemu-guest-agent is installed on the host so that after rootfs is shared into
the guest, our custom init (PID 1) process can directly run it as the one and
only "service" it manages.

The second big idea is that we use 9p filesystems to share host filesystem
inside the guest. This is useful so that vmtest targets can import/export data
in bulk without having to specify what to copy. In a kernel target, vmtest
exports two volumes: `/mnt/vmtest` and the root filesystem. The latter export
effectively gives the guest VM the same userspace environment as the host,
except we mount it read-only so the guest cannot do too much damage to the
host.

## Acknowledgements

Many thanks to [`drgn`'s
vmtest](https://github.com/osandov/drgn/tree/main/vmtest) by Omar Sandoval and
Andy Lutomirski's most excellent [`virtme`](https://github.com/amluto/virtme)
for providing both ideas and technical exploration.
