# vmtest

** Currently under active development. Feedback is welcome. **

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

* `qemu`(https://pkgs.org/download/qemu)
* [`qemu-guest-agent`](https://pkgs.org/search/?q=qemu-guest-agent)
* [OVMF](https://pkgs.org/download/ovmf) (if using `uefi =` target parameter)

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

```toml
[[target]]
name = "AWS kernel"
kernel = "./bzImage-5.15.0-1022-aws
command = "/bin/bash -c 'uname -r | grep -e aws$'"

[[target]]
name = "OCI image"
image = "./oci-stage-6/oci-stage-6-disk001.qcow2"
command = "/bin/ls -l /mnt/vmtest"
```

In the above config, two see two defined targets: "AWS kernel" and "OCI image".

In plain english, the "AWS kernel" target tells vmtest to run `command` in a VM
with the same userspace environment as the host, except with the specified
`kernel`.

"OCI image", on the other hand, tells vmtest to run `command` inside the
provided VM image. The image completely defines the environment `command` is
run in with the exception of `/mnt/vmtest`. `/mnt/vmtest` (as we will see
below) contains the full directory tree of the host machine rooted at the
directory containing `vmtest.toml`. This directory tree is shared - not copied
- with both readable and writable permissions.

Running vmtest with the above config yields the following results:

```
$ vmtest
Target 'AWS kernel' results:
============================
Exit code: 0
Stdout:
5.15.0-1022-aws-avx1


Target 'OCI image' results:
===========================
Exit code: 0
Stdout:
total 2057880
drwxr-xr-x 1 ubuntu ubuntu        200 Nov 14 20:41 oci-stage-6
-rw-r--r-- 1 ubuntu ubuntu   11631520 Feb  1 00:33 bzImage-5.15.0-1022-aws
-rw-r--r-- 1 ubuntu ubuntu        221 Nov 18 23:21 vmtest.toml
```

## Technical details

XXX include comms diagram
XXX note GHA works out of the box
