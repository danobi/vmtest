#!/usr/bin/env bash

set -eu

function die() {
  echo "$1" 1>&2
  exit 1
}

[[ $# == 2 ]] || die "incorrect args"
DISTRO="$1"
FILE="$2"

grep "ID=${DISTRO}" /etc/os-release &> /dev/null || die "bad distro"
test -f "/mnt/vmtest/${FILE}" || die "test file not found"
echo PASS
