#!/usr/bin/env bash

set -eu

function die() {
  echo "$1" 1>&2
  exit 1
}

[[ $# == 1 ]] || die "incorrect args"
DISTRO="$1"

grep "ID=${DISTRO}" /etc/os-release &> /dev/null || die "bad distro"
echo PASS
