#!/bin/bash
#
# Build a vmtest ready VM image.
#
# Run this on your root host inside vmtest repository.
#
# Usage:
#   ./scripts/build_image.sh not-uefi raw
#   ./scripts/build_image.sh uefi raw-efi

set -eu

function cleanup() {
  rm -rf .link
}

if [[ $# != 2 ]]; then
	echo "Usage: $0 <config> <format>"
	exit 1
fi

nix run github:nix-community/nixos-generators -- -f "$2" -c "./tests/images/${1}.nix" -o .link
trap cleanup EXIT

cp .link/* "./image-${1}.${2}"
chmod 666 "image-${1}.${2}"
