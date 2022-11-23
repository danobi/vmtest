MKOSI_CONFIGS := $(shell find tests -name 'mkosi.default')
MKOSI_DIRS := $(dir $(MKOSI_CONFIGS))
MKOSI_IMAGES := $(foreach dir,$(MKOSI_DIRS),$(dir)/image.raw)

.PHONY: all
all:
	@cargo build

.PHONY: test
test: images
	@RUST_LOG=debug cargo test -- --test-threads=1 --nocapture

.PHONY: images
images: $(MKOSI_IMAGES)

# Macro to define a target for each mkosi image
define mkosi_image
$(1)/image.raw: $(1)/mkosi.default
	@sudo mkosi -C $(1) --force;
endef

# Call macro
$(foreach dir,$(MKOSI_DIRS),$(eval $(call mkosi_image,$(dir))))
