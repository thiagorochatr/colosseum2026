# Deploying the Solana programs

The compliant pool is bound at build time to one compliance program, one curator and one denomination, so build after choosing all three. `make build-sbf` refuses to run unless `MIRROR_COMPLIANCE_PROGRAM_ID`, `MIRROR_REQUIRED_CURATOR` and `MIRROR_DENOMINATION` are set; `make build-test` is the build bound to the public test keys, for the tests only.

```sh
# 1. Keys. Program keys are needed again to upgrade; keep them.
solana-keygen new -o .demo/devnet/compliance-program.json
solana-keygen new -o .demo/devnet/pool-program.json
solana-keygen new -o .demo/devnet/curator.json

# 2. Build, bound to those ids, into a separate target directory.
MIRROR_COMPLIANCE_PROGRAM_ID=$(solana-keygen pubkey .demo/devnet/compliance-program.json) \
MIRROR_REQUIRED_CURATOR=$(solana-keygen pubkey .demo/devnet/curator.json) \
MIRROR_DENOMINATION=100000000 \
make build-sbf SBF_TARGET_DIR=$PWD/target/devnet

# 3. Deploy both.
solana program deploy -u devnet --program-id .demo/devnet/compliance-program.json target/devnet/deploy/compliance_program.so
solana program deploy -u devnet --program-id .demo/devnet/pool-program.json target/devnet/deploy/compliant_pool_program.so

# 4. Record the ids in deployments/devnet.json, then run the flow.
cd client && npm install && NETWORK=devnet node e2e.mjs
```

The build embeds the ids as constants. A check during this deployment found every 4-byte chunk of the devnet ids in the devnet binary and none of the test ids, and the reverse in the test binary.

For a local rehearsal, build against the test keys at the denomination in `deployments/local.json` (0.1 SOL; the test build serves 1 SOL, so InitPool would fail with 107, `PoolNotServed`), and load it at the ids that file names:

```sh
field() { node -p "require('./deployments/local.json').$1"; }
MIRROR_COMPLIANCE_PROGRAM_ID=$(solana-keygen pubkey tests/keys/compliance-program.json) \
MIRROR_REQUIRED_CURATOR=$(solana-keygen pubkey tests/keys/curator.json) \
MIRROR_DENOMINATION=$(field denomination) \
make build-sbf SBF_TARGET_DIR=$PWD/target/local
solana-test-validator --reset \
  --bpf-program $(field complianceProgram) target/local/deploy/compliance_program.so \
  --bpf-program $(field poolProgram) target/local/deploy/compliant_pool_program.so
# in another shell
cd client && NETWORK=local CURATOR_KEYPAIR=../tests/keys/curator.json node e2e.mjs
```
