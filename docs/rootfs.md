# Getting a rootfs

There are many ways to produce a directory to pass to the `rootfs` config field,
here are a couple of potential solutions.

## From a container image

OCI images can be turned into tarballs which can be extracted into a rootfs. For
example:

```sh
❯❯ mkdir $rootfs_dir && cd $rootfs_dir
❯❯ cat > Containerfile
FROM docker.io/library/debian
RUN apt update
RUN apt install -y qemu-guest-agent

❯❯ podman build -t deb-qga  # Docker would work exactly the same
❯❯ podman export -o deb.tar $(podman create deb-qga)
❯❯ tar xf deb.tar
❯❯ rm Containerfile deb.tar
```

## Using mkosi

[`mkosi`](https://github.com/systemd/mkosi) is a more advanced tool for building
OS images, as well as just producing a rootfs it can build full disk images with
a bootloader, plus many other features. You'll need to refer to the full
documentation to really understand `mkosi`, but here's a minimal example. This
will only work if you host system has `apt` (on Ubuntu you'll also need to
install the `debian-archive-keyring` package), otherwise you'll need to adapt it
for your host distro or run it in a container.

`mkosi.conf`:

```ini
[Output]
Format=directory

[Distribution]
Distribution=debian
Release=bookworm

[Content]
Packages=
        mount
        qemu-guest-agent
```

Then from the directory containing that file, run `mkosi -f`. This should
produce a directory named `image` that you can use for your `rootfs` config
field.