#!/bin/bash
#
# This serves as the init process (PID 1) of the guest VM.
#
# Hopefully this stays fairly simple. The basic idea is that the host rootfs is
# shared as read-only (see `ro` kernel cmdline param).  Since everything
# appears as a regular directory (and not a mount point), we are allowed to
# "mount over" any directory to make it guest-specific and possibly writable.
#
# In general, we want to mount over most of the major pseudo-fs's like procfs,
# cgroup2, devtmpfs, etc.
#
# And for any directories we want the guest to to be able to write into, we can
# mount a tmpfs over it.

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
    mkdir -p /dev
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

log "Mounting debugfs at /sys/kernel/debug"
mount -t debugfs debugfs /sys/kernel/debug || log "Failed to mount debugfs. CONFIG_DEBUG_FS might be missing from the kernel config"

log "Mounting tracefs at /sys/kernel/debug/tracing"
mount -t tracefs tracefs /sys/kernel/debug/tracing || log "Failed to mount tracefs. CONFIG_DEBUG_FS might be missing from the kernel config"

log "Mounting cgroup2 at /sys/fs/cgroup"
mount -t cgroup2 -o nosuid,nodev,noexec cgroup2 /sys/fs/cgroup

log "Mounting tmpfs at /mnt"
mount -t tmpfs -o nosuid,nodev tmpfs /mnt

# Symlink /dev/fd to /proc/self/fd so process substitution works.
log "Symlink /dev/fd to /proc/self/fd"
[[ -a /dev/fd ]] || ln -s /proc/self/fd /dev/fd

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
    log "Failed to locate qemu-guest-agent virtio-port"
    exit 1
fi
log "Located qemu-guest-agent virtio port: ${vport}"

# Send QGA logs out via kmsg if possible
qga_logs=
if [[ -e /dev/kmsg ]]; then
    qga_logs="--logfile /dev/kmsg"
fi

log "Spawning qemu-ga"
qemu-ga --method=virtio-serial --path="$vport" $qga_logs
