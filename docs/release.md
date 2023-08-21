# Release process

This document describes the `vmtest` release process.

## Steps

1. Install [`cargo-release`][0] if not already installed
1. Ensure master compiles and passes all tests
1. Check out `master` locally
1. Run `cargo release <LEVEL>`, where `<LEVEL>` could be `major`, `minor`, or
   `patch`, depending on the semantic changes
1. If all looks good, rerun with `--execute` to make it so
1. Check that [.github/workflows/release.yml][1] picks up the new tag and that
   a new [release][2] is created

[0]: https://github.com/crate-ci/cargo-release
[1]: ../.github/workflows/release.yml
[2]: https://github.com/danobi/vmtest/releases
