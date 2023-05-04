#!/bin/bash
#
# Build a vmtest ready kernel.
#
# Run this on your root host inside vmtest repository.
#
# Usage:
#   ./scripts/build_kernel.sh v6.2

set -eu

# Go to repo root
cd "$(git rev-parse --show-toplevel)"

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <kernel-tag>"
    exit 1
fi

# Build builder
docker build \
    --build-arg KERNEL_TAG="$1" \
    -t vmtest-kernel-builder-"$1" \
    -f scripts/docker/Dockerfile.kernel \
    scripts/docker

# Run builder
docker run -v "$(pwd):/output" -it vmtest-kernel-builder-"$1"
