# Frequently asked questions

### I'm getting errors about "read-only file system"

If you are using a `kernel` target, `vmtest` by default mounts the root
filesystem as read-only to protect any misbehaving tests from interfering with
your host. Only the current directory that `vmtest` is run from is mounted
readable/writable at `/mnt/vmtest`.

However, if you know better and would like to override this behavior, add `rw`
to the `kernel_args` target config and the root filesystem will be readable and
writable.
