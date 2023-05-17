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
* `command` (string)
    * Required field
    * Command to run inside VM
    * Note that the specified command is run inside a `bash` shell by default
