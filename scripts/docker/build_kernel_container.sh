#!/bin/bash
#
# Build kernel from inside container.
#
# Note this scripts expects the bind mounted output directly to be mounted
# at /output.

set -eux

make -j "$(nproc)" bzImage
bzimage=$(find . -type f -name bzImage)
cp "$bzimage" /output
