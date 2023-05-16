# Test documentation

## Image tests

Currently the CI builds complete OS images using `mkosi` on demand and runs
vmtest against the build images in a few configurations. This is all
orchestrated by `test.rs`.

## Kernel tests

To test kernels, we have a [GHA workflow][0] that builds kernels based off
[`KERNELS`][1] and uploads them to the [`test_assets`][2] dummy release as
release assets. These release assets can then be downloaded over HTTP.

TODO(danobi): the actual tests that use the uploaded kernels still need to
be written.


[0]: https://github.com/danobi/vmtest/actions/workflows/kernels.yml
[1]: ./KERNELS
[2]: https://github.com/danobi/vmtest/releases/tag/test_assets
