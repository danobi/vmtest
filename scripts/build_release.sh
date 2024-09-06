#!/bin/bash
#
# Build a release version of vmtest for a list of architectures.
# The resulting binaries will be copied in the current directory and named vmtest-<arch>.
#
# Run this on your root host inside vmtest repository.
#
# Note: for foreign architectures we will emulate using qemu. This can be slow.
#
# Usage:
#   ./scripts/build_release.sh
#   ./scripts/build_release.sh x86_64 aarch64

set -eu

ARCHS=(x86_64 aarch64 s390x)

# Allow user to pick subset
if [[ $# -gt 0 ]]
then
    ARCHS=("$@")
fi

declare -A ARCH_TO_IMAGE_ARCH=(
  [x86_64]=amd64
  [aarch64]=arm64v8
  [s390x]=s390x
)

declare -A ARCH_TO_DOCKER_ARCH=(
  [x86_64]=linux/amd64
  [aarch64]=linux/arm64
  [s390x]=linux/s390x
)

# Install binfmt hooks - very magical (but is necessary)
docker run --rm --privileged tonistiigi/binfmt --install all

for arch in "${ARCHS[@]}"; do
  image_name="vmtest-release-${arch}"

  docker build \
    --platform "${ARCH_TO_DOCKER_ARCH[${arch}]}" \
    --build-arg IMAGE_ARCH="${ARCH_TO_IMAGE_ARCH[$arch]}" \
    --build-arg TARGET_ARCH="${arch}" \
    -f scripts/docker/Dockerfile.release \
    -t "${image_name}" \
    .

  # Need to create a container to copy from it
  id=$(docker create "${image_name}")
  docker cp "${id}:/output/vmtest-${arch}" .
  docker rm -v "${id}"
done
