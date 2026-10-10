#!/bin/sh
# Regenerates every verifier and fixture from one proving key, so both chains
# always verify the same statement under the same key.
#
#   sh scripts/regenerate-verifiers.sh <zkey> <vk.json> "<label for the generated Solana module>"
set -eu
cd "$(dirname "$0")/.."
ZKEY=${1:?zkey}; VK=${2:?vk.json}; LABEL=${3:?label}
ROOT=$(cd .. && pwd)

# Proofs of every valid vector under this key, then their Solidity calldata.
node scripts/prove-fixtures.mjs "$ZKEY" build/proofs.json
node scripts/solidity-fixtures.mjs build/proofs.json "$ROOT/tempo/test/fixtures/proofs.json"

# Tempo: the Solidity verifier, straight from snarkjs.
npx snarkjs zkey export solidityverifier "$ZKEY" "$ROOT/tempo/src/Groth16Verifier.sol"

# Solana: the verifying-key module and the on-chain fixtures. The bridge refuses
# to emit any proof that arkworks does not verify under this key.
BRIDGE="cargo run -q --manifest-path $ROOT/solana/Cargo.toml -p snarkjs-bridge --"
$BRIDGE vk "$VK" "$LABEL" > "$ROOT/solana/programs/compliance/src/vk.rs"
$BRIDGE proofs "$VK" build/proofs.json > "$ROOT/solana/programs/compliance/tests/fixtures/proofs.json"
cp "$VK" "$ROOT/solana/crates/snarkjs-bridge/tests/fixtures/vk.json"
cp build/proofs.json "$ROOT/solana/crates/snarkjs-bridge/tests/fixtures/proofs.json"

echo "verifying key sha256: $(shasum -a 256 "$VK" | cut -d' ' -f1)"
grep "Key digest" "$ROOT/solana/programs/compliance/src/vk.rs"
