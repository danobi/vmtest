# For images and kernels, see https://github.com/danobi/vmtest/releases/tag/test_assets for available assets.
IMAGES := image-not-uefi.raw image-uefi.raw-efi
KERNELS := bzImage-v5.15-empty bzImage-v6.2-empty

ASSET_DIRECTORY := tests/.assets
IMAGES_FILES := $(foreach image,$(IMAGES),$(ASSET_DIRECTORY)/$(image))
KERNELS_FILES := $(foreach kernel,$(KERNELS),$(ASSET_DIRECTORY)/$(kernel))

.PHONY: all
all:
	@cargo build

.PHONY: test
test: $(IMAGES_FILES) $(KERNELS_FILES)
	@RUST_LOG=debug cargo test -- --test-threads=1 --nocapture

.PHONY: clean
clean:
	@cargo clean
	@rm -rf $(ASSET_DIRECTORY)

$(ASSET_DIRECTORY):
	@mkdir -p $@

$(IMAGES_FILES): | $(ASSET_DIRECTORY)
	@curl -q -L https://github.com/danobi/vmtest/releases/download/test_assets/$(notdir $@).zst -o $@.zst
	@zstd -d $@.zst -o $@
	@rm $@.zst

$(KERNELS_FILES): | $(ASSET_DIRECTORY)
	@curl -q -L https://github.com/danobi/vmtest/releases/download/test_assets/$(notdir $@) -o $@
