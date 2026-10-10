.PHONY: all build-sbf test verify

# The SBF build gets its own target directory, for the reason mirror-pool's
# Makefile gives: host proc-macro artifacts in a shared directory break it.
SBF_TARGET_DIR := $(CURDIR)/target/sbf

all: verify

build-sbf:
	CARGO_TARGET_DIR=$(SBF_TARGET_DIR) cargo build-sbf --manifest-path programs/compliance/Cargo.toml
	@ls -l $(SBF_TARGET_DIR)/deploy/*.so

test:
	cargo test --workspace

verify: build-sbf test
