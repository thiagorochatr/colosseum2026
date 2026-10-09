.PHONY: all test lint fmt build-sbf clean verify

# cargo build-sbf compiles for sbf-solana-solana, but proc-macro dependencies
# still have to be built for the host. If the shared target directory already
# holds host artifacts from `cargo test` or `cargo clippy`, cargo can hand the
# SBF rustc a host .dylib for one of those macros, which it rejects with
# "extern location ... is of an unknown type". Giving the SBF build its own
# target directory removes the collision.
#
# The program deliberately stays a member of the root workspace so that
# `cargo test --workspace` covers it. Excluding it from the workspace is the
# usual way to dodge the target-directory collision above, and it silently
# removes every on-chain test from CI: the suite still passes, because it no
# longer runs.
#
# The path must be absolute. cargo resolves a relative CARGO_TARGET_DIR against
# the manifest directory, so `target/sbf` with `--manifest-path programs/...`
# builds into programs/mirror-pool/target/sbf while cargo-build-sbf's
# post-processing looks for the artifact under the workspace root, and the build
# "succeeds" while producing no .so.
SBF_TARGET_DIR := $(CURDIR)/target/sbf

all: verify

fmt:
	cargo fmt --all

lint:
	cargo fmt --all -- --check
	cargo clippy --workspace --all-targets -- -D warnings

test:
	cargo test --workspace --all-features

build-sbf:
	CARGO_TARGET_DIR=$(SBF_TARGET_DIR) cargo build-sbf --manifest-path programs/mirror-pool/Cargo.toml
	@ls -l $(SBF_TARGET_DIR)/deploy/*.so

# Everything CI runs, in the order CI runs it.
#
# build-sbf comes before test, and the order is load-bearing: the end-to-end
# suite loads the .so this produces into a real SVM, and the artifact is
# gitignored. With test first, `make verify` fails on a fresh clone at the one
# command the README tells a reader to run.
verify: lint build-sbf test

clean:
	cargo clean
	rm -rf $(SBF_TARGET_DIR)
