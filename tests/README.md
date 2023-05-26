# Test documentation

## Image tests

Images are built using `./scripts/build_image.sh`. Here's an example invocation:

```
$ ./scripts/build_image.sh not-uefi raw
```

It'll dump out in your current directory a file named `image-not-uefi.raw`.
Under the hood it uses `nix`/`nixos` to build bootable images. All you need
is to have [nix][3] installed to run it.

Tests in `test.rs` use download images from [test_assets][2] and run `vmtest`
against the images. Asset downloads are orchestrated by the [Makefile][4].

### Updating images

Currently we build the images locally and upload them to [test_assets][2] by
hand. We _would_ build the images in the CI like with the kernels, but nixos
requires KVM in order to function. Since free GHA runners do not support nested
virt, we must build locally.

Before uploading, first compress the image with `zstd`. This is necessary to
avoid the 2G upload limit.

```
$ zstd image-not-uefi.raw
$ gh release upload test_assets ./image-not-uefi.raw.zst
```

## Kernel tests

To test kernels, we have a [GHA workflow][0] that builds kernels based off
[`KERNELS`][1] and uploads them to the [`test_assets`][2] dummy release as
release assets. These release assets can then be downloaded over HTTP.

Similar to the image tests, `make test` orchestrates asset downloads before
running the test suite.


[0]: https://github.com/danobi/vmtest/actions/workflows/kernels.yml
[1]: ./KERNELS
[2]: https://github.com/danobi/vmtest/releases/tag/test_assets
[3]: https://nixos.org/download.html
[4]: ../Makefile
