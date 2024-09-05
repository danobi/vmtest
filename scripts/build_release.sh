#!/bin/bash
#
# Build a release version of vmtest for a list of architectures.
# The resulting binaries will be copied in the current directory and named vmtest-<arch>.
#
# Run this on your root host inside vmtest repository.
#
# Usage:
#   ./scripts/build_release.sh
#   ./scripts/build_release.sh x86_64 aarch64

set -eu

# Go to repo root
cd "$(git rev-parse --show-toplevel)"

docker build \
  -t vmtest-release-builder \
  -f scripts/docker/Dockerfile.release \
  .

docker run --rm -v $(pwd)/output" vmtest-release-builder "$@"
