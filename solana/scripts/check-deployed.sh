#!/bin/sh
# Rebuilds both programs bound to the devnet ids in deployments/devnet.json and
# checks that devnet runs exactly these bytes.
#
#   sh scripts/check-deployed.sh          (RPC from $SOLANA_RPC, else public devnet)
#
# The build is reproducible on the platform it was deployed from, macOS arm64:
# platform-tools ships a prebuilt standard library whose panic locations carry
# the path it was built under, so a Linux build differs in those strings.
set -eu
cd "$(dirname "$0")/.."
RPC=${SOLANA_RPC:-https://api.devnet.solana.com}
field() { node -p "JSON.parse(require('fs').readFileSync('deployments/devnet.json','utf8')).$1"; }

export MIRROR_COMPLIANCE_PROGRAM_ID="$(field complianceProgram)"
export MIRROR_REQUIRED_CURATOR="$(field curatorAuthority)"
OUT=$PWD/target/devnet
make build-sbf SBF_TARGET_DIR="$OUT" >/dev/null

DUMP=$(mktemp -d)
status=0
for pair in "compliance_program:$(field complianceProgram)" "compliant_pool_program:$(field poolProgram)"; do
  name=${pair%%:*}; id=${pair#*:}
  solana program dump -u "$RPC" "$id" "$DUMP/$name.so" >/dev/null
  # Program data is allocated at deploy time and may be longer than the
  # program: the deployed bytes must be the build followed by zeros only.
  if node scripts/same-program.mjs "$OUT/deploy/$name.so" "$DUMP/$name.so"; then
    echo "$name $id: devnet runs this build ($(shasum -a 256 "$OUT/deploy/$name.so" | cut -c1-16)...)"
  else
    status=1
  fi
done
exit $status
