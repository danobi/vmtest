# Frequently asked questions

### Why am I getting errors about "read-only file system"

If you are using a `kernel` target, `vmtest` by default mounts the root
filesystem as read-only to protect any misbehaving tests from interfering with
your host. Only the current directory that `vmtest` is run from is mounted
readable/writable at `/mnt/vmtest`.

However, if you know better and would like to override this behavior, add `rw`
to the `kernel_args` target config and the root filesystem will be readable and
writable.

### Why is vmtest so slow on my machine?

`vmtest` relies on hardware acceleration for performance. It will, however,
fall back to emulation if hardware acceleration is not available. If vmtest is
running your code abnormally slow, then check if your host supports KVM. One
easy way to check is by seeing if `/dev/kvm` exists.

### Why is vmtest slow in Github Actions?

`vmtest` relies on hardware acceleration for performance. [Standard Github
Action runners][0] do not support nested virtualization. Meaning that vmtest
cannot take advantage of hardware acceleration (see above "Why is vmtest so
slow on my machine?").

The good news is that Github now supports nested virtualization on [large
runners][1]. Unfortunately large runners are currently only available for paid
plans, so most open source projects will not be able to take advantage of
nested virtualization.

### How do I run docker inside vmtest?

Docker is quite tricky to run inside vmtest b/c it requires a lot of mutable
host state as well as communication over a unix domain socket to the `dockerd`
daemon. Unix domain sockets cannot be shared across host and guest VM. `dockerd`
does, however, support TCP sockets. But that is tricky as well b/c now you need
to bring up networking inside the guest VM.

Ultimately, I don't think it's worth the trouble to set up docker inside the
guest. Rather, prefer to run vmtest inside the docker container. This workflow
is supported and works quite well.

### How do I run podman inside vmtest?

In contrast to docker, podman is daemonless, meaning it should avoid the issues
that docker has with vmtest, right? Kind of.

Podman still suffers from relying on mutable host state. This can be worked
around by adding `rw` to the kernel command line to make the rootfs mutable.

Podman by default (at time of writing) also relies on overlayfs to achieve
rootless container execution. B/c of how vmtest is designed, all kernel modules
need to be built into bzImage for vmtest guests to use them. While in theory it
is possible to track down all the configs podman requires, I didn't go down
that rabbit hole far enough.

Instead, prefer to run vmtest inside the podman container, just like with
docker.


[0]: https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners#supported-runners-and-hardware-resources
[1]: https://github.blog/changelog/2023-02-23-hardware-accelerated-android-virtualization-on-actions-windows-and-linux-larger-hosted-runners/
