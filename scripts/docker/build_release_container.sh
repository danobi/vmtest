#!/bin/bash
#
# Build release binaries from inside container.
#
# Note this script expects the bind mounted output directly to be mounted
# at /output.

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

if (( ${#X_ARCHS[@]} > 0 )); then
  apt-get update
  for arch in "${X_ARCHS[@]}"; do
    apt-get install -y "gcc-${arch//_/-}-linux-gnu"
    rustup target add "${arch}-unknown-linux-gnu"
  done
fi


for arch in "${ARCHS[@]}"; do
  # Tell dependencies to link statically
  export LIBCAPNG_LINK_TYPE=static
  export LIBCAPNG_LIB_PATH="/usr/lib/${arch}-linux-gnu"
  export LIBSECCOMP_LIB_TYPE=static

  # Compile the binary
  RUSTFLAGS="-C target-feature=+crt-static -C linker=/usr/bin/${arch}-linux-gnu-gcc" cargo build --release --target "${arch}-unknown-linux-gnu"
  cp "./target/${arch}-unknown-linux-gnu/release/vmtest" "/output/vmtest-${arch}"
done
