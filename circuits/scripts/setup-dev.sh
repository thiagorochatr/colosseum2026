#!/bin/sh
# A DEVELOPMENT key: one local contribution on top of the public phase-1 file.
# It unblocks testing and is never deployed as final. The real key comes from
# the multi-party ceremony in CEREMONY.md.
set -eu
cd "$(dirname "$0")/.."
PTAU=ptau/ppot_0080_14.ptau
[ -f "$PTAU" ] || { echo "missing $PTAU, see ptau/README.md"; exit 1; }
npx snarkjs groth16 setup build/withdraw.r1cs "$PTAU" build/withdraw_dev_0.zkey
npx snarkjs zkey contribute build/withdraw_dev_0.zkey build/withdraw_dev.zkey \
  --name="development only" -e="$(head -c 64 /dev/urandom | od -An -tx1 | tr -d ' \n')"
npx snarkjs zkey verify build/withdraw.r1cs "$PTAU" build/withdraw_dev.zkey
npx snarkjs zkey export verificationkey build/withdraw_dev.zkey build/vk_dev.json
