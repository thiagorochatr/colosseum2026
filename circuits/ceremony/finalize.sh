#!/bin/sh
# Applies the public beacon, verifies the whole chain, and exports the key.
#
#   sh ceremony/finalize.sh <announced slot>
set -eu
cd "$(dirname "$0")/.."
SLOT=${1:?usage: finalize.sh <announced slot>}
LAST=$(ls ceremony/withdraw_[0-9][0-9][0-9][0-9].zkey | sort | tail -1)
BEACON=$(node ceremony/solana-beacon.mjs "$SLOT")
echo "$BEACON" > ceremony/beacon.json
HEX=$(echo "$BEACON" | node -e 'process.stdin.on("data",d=>console.log(JSON.parse(d).hex))')
USED=$(echo "$BEACON" | node -e 'process.stdin.on("data",d=>console.log(JSON.parse(d).slot))')
npx snarkjs zkey beacon "$LAST" ceremony/withdraw_final.zkey "$HEX" 10 \
  -n="Solana mainnet blockhash at slot $USED (announced slot $SLOT)"
# Without pipefail, `verify | tee` would hide a failed verification: check the transcript instead.
# snarkjs colours its log even into a pipe; the transcript, as CI compares it, has no colour codes.
npx snarkjs zkey verify build/withdraw.r1cs ptau/ppot_0080_14.ptau ceremony/withdraw_final.zkey | sed 's/\x1b\[[0-9;]*m//g' | tee ceremony/transcript.txt
grep -q "ZKey Ok!" ceremony/transcript.txt || { echo "the final key does not verify" >&2; exit 1; }
npx snarkjs zkey export verificationkey ceremony/withdraw_final.zkey ceremony/vk.json
shasum -a 256 ceremony/withdraw_final.zkey ceremony/vk.json
