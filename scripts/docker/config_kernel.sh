#!/bin/bash
#
# Configure a vmtest ready kernel.
#
# Run this inside kernel source tree root.

set -eux

# Start with a distro config
cp "distros/${1}" .config

# If an empty config was provided, then we need to start with defconfig
# to get a sane config. Otherwise, all we need to do is default out the
# new or unset configs.
if [[ -s .config ]]; then
	make olddefconfig
else
	make defconfig
fi

# Apply vmtest required configs
./scripts/config \
	-e VIRTIO \
	-e VIRTIO_PCI \
	-e VIRTIO_CONSOLE \
	-e NET_9P \
	-e NET_9P_VIRTIO \
	-e 9P_FS

# Disable x86 insn decoder selftest. It takes way too long to run.
./scripts/config -d X86_DECODER_SELFTEST

# Setting previous configs may result in more sub options being available,
# so set all the new available ones to default as well.
make olddefconfig
