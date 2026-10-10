#!/bin/sh
# Compiles the circuit into build/: the R1CS, the witness generator (wasm) and the symbols.
set -eu
cd "$(dirname "$0")/.."
mkdir -p build
circom withdraw.circom --O2 --r1cs --wasm --sym -l node_modules -o build
npx snarkjs r1cs info build/withdraw.r1cs
