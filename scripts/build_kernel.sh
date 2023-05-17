#!/bin/bash
#
# Build a vmtest ready kernel.
#
# Run this on your root host inside vmtest repository.
#
# Usage:
#   ./scripts/build_kernel.sh v6.2
#   ./scripts/build_kernel.sh v6.2 archlinux

set -eu

# Go to repo root
cd "$(git rev-parse --show-toplevel)"

if [[ $# < 1 || $# > 2 ]]; then
	echo "Usage: $0 <kernel-tag> [<distro>]"
	exit 1
fi

# Use empty config file if no distro is specified
DISTRO=${2:-empty}

# Unique identifier for the kernel being built
IDENTIFIER="$1"-"$DISTRO"

# Build builder
docker build \
	--build-arg KERNEL_TAG="$1" \
	--build-arg DISTRO="$DISTRO" \
	-t vmtest-kernel-builder-"$IDENTIFIER" \
	-f scripts/docker/Dockerfile.kernel \
	scripts/docker

# Run builder
docker run --rm -v "$(pwd):/output" vmtest-kernel-builder-"$IDENTIFIER"

# Rename bzImage appropriately
mv -f bzImage bzImage-"$IDENTIFIER"
