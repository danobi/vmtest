IMAGES := fedora37 ubuntu22
IMAGES_FILES := $(foreach image,$(IMAGES),tests/.assets/image-$(image).raw)
ASSET_DIRECTORY := tests/.assets

.PHONY: all
all:
	@cargo build

.PHONY: test
test: $(IMAGES_FILES)
	@RUST_LOG=debug cargo test -- --test-threads=1 --nocapture

.PHONY: clean
clean:
	@cargo clean
	@rm -rf $(ASSET_DIRECTORY)

$(ASSET_DIRECTORY):
	@mkdir -p $@

$(IMAGES_FILES): | tests/.assets
	@curl -q -L https://github.com/danobi/vmtest/releases/download/test_assets/$(notdir $@) -o $@
