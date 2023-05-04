#!/bin/bash
#
# Configure a vmtest ready kernel.
#
# Run this inside kernel source tree root.

set -eux

# Start with all defaults
make defconfig

# Apply vmtest required configs
./scripts/config \
    -e VIRTIO \
    -e VIRTIO_PCI \
    -e NET_9P \
    -e NET_9P_VIRTIO \
    -e 9P_FS

# Setting previous configs may result in more sub options being available,
# so set all the new available ones to default as well.
make olddefconfig
