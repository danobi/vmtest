# Configuration

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
* `kernel_args` (string)
    * Optional field
    * `kernel` must be specified
    * Additional kernel command line arguments to append to `vmtest` generated
      kernel arguments
* `rootfs` (string)
    * Default: `/`
    * `kernel` must be specified
    * Path to rootfs to test against
    * If a relative path is provided, it will be interpreted as relative to
      `vmtest.toml`
* `arch` (string)
    * Default: the architecture vmtest was built for.
    * Under which machine architecture to run the kernel.
* `command` (string)
    * Required field
    * Command to run inside VM
    * Note that the specified command is run inside a `bash` shell by default
    * `vmtest`'s environment variables are also propagated into the VM during
      command execution
* `vm` (VMConfig)
    * Optional sub-table
    * Configures the VM.
    * See the VMConfig struct below.

### `[[target.vm]]`

The VMConfig struct that configures the QEMU VM.

* `num_cpus` (int)
    * Optional field
    * Number of CPUs in the VM.
    * Default: 2
* `memory` (string)
    * Optional field
    * Amount of RAM for the VM.
    * Accepts a QEMU parsable string for the -m flag like 256M or 4G.
    * Default: 4G
* `mounts` (Map<String, Mount>)
    * Optional sub-table
    * Map of additional host mounts for the VM.
    * Key is the path in the VM and the value contains information about the host path.
    * See below for definition of the Mount object.
* `bios` (string)
    * Optional field
    * Path to the BIOS file.
    * This is only used if the UEFI flag from target is true.
* `extra_args` (List<string>)
    * Optional field
    * Extra arguments to pass to QEMU.

### `[[target.vm.mounts]]`

The Mount struct for defining additional host mounts into the VM.

* `host_path` (string)
    * Required field
    * Path on the host.
* `writable` (bool)
    * Optional field
    * Whether this mount is writable in the VM.
    * Default: false
