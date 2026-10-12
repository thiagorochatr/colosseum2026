.PHONY: all build-sbf test lint verify

# The SBF build gets its own target directory, for the reason mirror-pool's
# Makefile gives: host proc-macro artifacts in a shared directory break it.
SBF_TARGET_DIR := $(CURDIR)/target/sbf

# The test build of compliant-pool is bound to public test keys. A deployment
# builds with its own ids; see DEPLOY.md.
export MIRROR_COMPLIANCE_PROGRAM_ID ?= $(shell solana-keygen pubkey tests/keys/compliance-program.json)
export MIRROR_REQUIRED_CURATOR ?= $(shell solana-keygen pubkey tests/keys/curator.json)
# The one pool a build serves; the tests use 1 SOL.
export MIRROR_DENOMINATION ?= 1000000000

all: verify

build-sbf:
	CARGO_TARGET_DIR=$(SBF_TARGET_DIR) cargo build-sbf --manifest-path programs/compliance/Cargo.toml
	CARGO_TARGET_DIR=$(SBF_TARGET_DIR) cargo build-sbf --manifest-path programs/compliant-pool/Cargo.toml
	@ls -l $(SBF_TARGET_DIR)/deploy/*.so

test:
	cargo test --workspace

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

verify: lint build-sbf test
