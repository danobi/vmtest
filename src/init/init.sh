#!/bin/bash

set -eu

export PATH=/bin:/sbin:/usr/bin:/usr/sbin

log() {
    if [[ -e /dev/kmsg ]]; then
	echo "<6>vmtest: $*" >/dev/kmsg
    else
	echo "vmtest: $*"
    fi
}

# We start with the host procfs mounted in guest /proc.
#
# This may confuse some tools, so start off with mounting guest
# procfs at guest /proc.
/bin/mount -t proc -o nosuid,nodev,noexec proc /proc

# So the kernel doesn't panic when if we exit
trap 'poweroff -f' EXIT

umask 022

# devtmpfs might be automounted through CONFIG_DEVTMPFS_MOUNT.
# Check if it's already mounted, and if not, mount it.
#
# Note we do the check in a kind of hacky way to keep the output
# silent. We cannot rely on redirecting output to /dev/null yet.
if ! mount | grep -q " /dev "; then
    log "devtmpfs not automounted, mounting at /dev"
    mkdir /dev
    mount -t devtmpfs -o nosuid,noexec dev /dev
fi

if [[ ! -d /dev/shm ]]; then
    log "Mounting tmpfs at /dev/shm"
    mkdir /dev/shm
    mount -t tmpfs -o nosuid,nodev tmpfs /dev/shm
fi

log "Mounting tmpfs at /tmp"
mount -t tmpfs -o nosuid,nodev tmpfs /tmp

log "Mounting tmpfs at /run"
mount -t tmpfs -o nosuid,nodev tmpfs /run
ln -s /var/run ../run

log "Mounting sysfs at /sys"
mount -t sysfs -o nosuid,nodev,noexec sys /sys

log "Mounting cgroup2 at /sys/fs/cgroup"
mount -t cgroup2 -o nosuid,nodev,noexec cgroup2 /sys/fs/cgroup

log "Init done"

# Locate our QGA virtio port
vport=
for dir in /sys/class/virtio-ports/*; do
    if [[ "$(cat "$dir/name")" == "org.qemu.guest_agent.0" ]]; then
        vport_name=$(basename "$dir")
        vport="/dev/${vport_name}"
    fi
done
if [[ -z "$vport" ]]; then
    log "Failed to locate qemu-guest-agent virio-port"
    exit 1
fi
log "Located qemu-guest-agent virtio port: ${vport}"

log "Spawning qemu-ga"
qemu-ga --method=virtio-serial --path="$vport"
