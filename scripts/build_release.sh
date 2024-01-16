#!/bin/bash
#
# Build a release version of vmtest for a list of architectures.
# The resulting binaries will be copied in the current directory and named vmtest-<arch>.
#
# This script assumes it is run on a Debian based system.
#
# Run this on your root host inside vmtest repository.
#
# Usage:
#   ./scripts/build_release.sh
#   ./scripts/build_release.sh x86_64 aarch64

set -eu

ARCHS=(x86_64 aarch64 s390x)

if [[ $# -gt 0 ]]
then
    ARCHS=("$@")
fi

# Install the required toolchain for cross-compilation
X_ARCHS=()
for arch in "${ARCHS[@]}"; do
    if [[ "${arch}" == "$(uname -m)" ]]; then
        continue
    fi
    X_ARCHS+=("${arch}")
done

ARCHS_TO_EXPAND=$(IFS=, ; echo "${X_ARCHS[*]}")

sudo apt update
eval sudo apt install -y "gcc-{${ARCHS_TO_EXPAND//_/-}}-linux-gnu"
eval rustup target add "{${ARCHS_TO_EXPAND}}-unknown-linux-gnu"

for arch in "${ARCHS[@]}"; do
    # Compile the binary
    RUSTFLAGS="-C target-feature=+crt-static -C linker=/usr/bin/${arch}-linux-gnu-gcc" cargo build --release --target "${arch}-unknown-linux-gnu"
    cp "./target/${arch}-unknown-linux-gnu/release/vmtest" "./vmtest-${arch}"
done
