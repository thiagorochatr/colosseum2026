.PHONY: all build-sbf build-test test lint verify

# The SBF build gets its own target directory, for the reason mirror-pool's
# Makefile gives: host proc-macro artifacts in a shared directory break it.
SBF_TARGET_DIR := $(CURDIR)/target/sbf

# compliant-pool is bound at build time to a compliance program, a curator and
# a denomination. build-sbf builds a deployment and needs all three set
# explicitly (see DEPLOY.md). Every other target binds the public test keys,
# whose secrets are committed, so nothing built here by default is deployable.
BOUND_TO := MIRROR_COMPLIANCE_PROGRAM_ID MIRROR_REQUIRED_CURATOR MIRROR_DENOMINATION
# The tests use 1 SOL.
TEST_IDS = MIRROR_COMPLIANCE_PROGRAM_ID=$(shell solana-keygen pubkey tests/keys/compliance-program.json) \
	MIRROR_REQUIRED_CURATOR=$(shell solana-keygen pubkey tests/keys/curator.json) \
	MIRROR_DENOMINATION=1000000000

all: verify

build-sbf:
	@for v in $(BOUND_TO); do \
	  [ -n "$$(printenv $$v)" ] || { echo "build-sbf: set $$v (see DEPLOY.md), or run make build-test" >&2; exit 1; }; \
	done
	CARGO_TARGET_DIR=$(SBF_TARGET_DIR) cargo build-sbf --manifest-path programs/compliance/Cargo.toml
	CARGO_TARGET_DIR=$(SBF_TARGET_DIR) cargo build-sbf --manifest-path programs/compliant-pool/Cargo.toml
	@ls -l $(SBF_TARGET_DIR)/deploy/*.so

# The build the tests load, from target/sbf.
build-test:
	$(MAKE) build-sbf $(TEST_IDS)

# cargo compiles compliant-pool for the host too, so it needs the same ids.
test:
	$(TEST_IDS) cargo test --workspace

lint:
	cargo fmt --all -- --check
	$(TEST_IDS) cargo clippy --workspace --all-targets -- -D warnings

verify: lint build-test test
