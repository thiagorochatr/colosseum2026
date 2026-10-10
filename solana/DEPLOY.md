# Deploying the Solana programs

The compliant pool is bound at build time to one compliance program and one curator, so build after choosing both.

```sh
# 1. Keys. Program keys are needed again to upgrade; keep them.
solana-keygen new -o .demo/devnet/compliance-program.json
solana-keygen new -o .demo/devnet/pool-program.json
solana-keygen new -o .demo/devnet/curator.json

# 2. Build, bound to those ids, into a separate target directory.
MIRROR_COMPLIANCE_PROGRAM_ID=$(solana-keygen pubkey .demo/devnet/compliance-program.json) \
MIRROR_REQUIRED_CURATOR=$(solana-keygen pubkey .demo/devnet/curator.json) \
make build-sbf SBF_TARGET_DIR=$PWD/target/devnet

# 3. Deploy both.
solana program deploy -u devnet --program-id .demo/devnet/compliance-program.json target/devnet/deploy/compliance_program.so
solana program deploy -u devnet --program-id .demo/devnet/pool-program.json target/devnet/deploy/compliant_pool_program.so

# 4. Record the ids in deployments/devnet.json, then run the flow.
cd client && npm install && NETWORK=devnet node e2e.mjs
```

The build embeds the ids as constants. A check during this deployment found every 4-byte chunk of the devnet ids in the devnet binary and none of the test ids, and the reverse in the test binary.

For a local rehearsal, load the test build into `solana-test-validator` with `--bpf-program`, at the compliance test id from `tests/keys/`, and run with `NETWORK=local CURATOR_KEYPAIR=../tests/keys/curator.json`.
