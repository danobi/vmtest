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

for arch in "${ARCHS[@]}"; do
  echo "$arch" > /etc/apk/arch
  apk add --no-cache --allow-untrusted libcap-ng-static libseccomp-static
  rustup target add "${arch}-unknown-linux-musl"

  # Tell dependencies to link statically
  export LIBCAPNG_LINK_TYPE=static
  export LIBCAPNG_LIB_PATH="/usr/lib/"
  export LIBSECCOMP_LIB_TYPE=static

  # Compile the binary
  RUSTFLAGS="-C linker=/usr/bin/ld.lld" cargo build --release --target "${arch}-unknown-linux-musl"
  cp "./target/${arch}-unknown-linux-musl/release/vmtest" "/output/vmtest-${arch}"
done
