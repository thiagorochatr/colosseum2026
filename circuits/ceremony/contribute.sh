#!/bin/sh
# Adds one phase-2 contribution on top of a key in this directory.
#
#   sh ceremony/contribute.sh "Your name"                       on top of the latest numbered key
#   sh ceremony/contribute.sh "Your name" withdraw_final.zkey   on top of the final key, from the release
#
# The new key is withdraw_NNNN.zkey, NNNN being its own contribution number:
# the base key's count plus one, read from the key, not from the files here.
#
# Run it in your own terminal. It asks for some random text, mixes it with 64
# bytes from /dev/urandom, and hands the result to snarkjs. The entropy is never
# printed, never written to disk, and never passed as an argument or to a prompt. Your contribution is safe as long as that
# entropy is gone when the script exits.
set -eu
cd "$(dirname "$0")"
NAME=${1:?usage: contribute.sh "Your name" [base.zkey]}
# Find the base key and the output name before asking for any entropy.
LAST=${2:-$(ls withdraw_[0-9][0-9][0-9][0-9].zkey 2>/dev/null | sort | tail -1)}
if [ -z "$LAST" ] || [ ! -f "$LAST" ]; then
  echo "No key to contribute on. Download withdraw_final.zkey from the ceremony-v1 release into $(pwd) and pass it as the second argument." >&2
  exit 1
fi
# The contribution count is in the key's MPC section (10), after a 64-byte hash.
N=$(node -e '
const b = require("fs").readFileSync(process.argv[1]);
if (b.toString("latin1", 0, 4) !== "zkey") throw new Error("not a zkey file");
for (let i = 0, o = 12; i < b.readUInt32LE(8); i++, o += 12 + Number(b.readBigUInt64LE(o + 4))) {
  if (b.readUInt32LE(o) === 10) { console.log(b.readUInt32LE(o + 12 + 64)); process.exit(0); }
}
throw new Error("no contributions section");
' "$LAST")
NEXT=$(printf "withdraw_%04d.zkey" "$(expr "$N" + 1)")
if [ -e "$NEXT" ]; then
  echo "$NEXT already exists. Move it away, or contribute on top of it." >&2
  exit 1
fi
printf "Type some random text, then Enter (it is not shown): "
# Turn echo back on however the script ends, Ctrl-C included.
if [ -t 0 ]; then trap 'stty echo' EXIT INT TERM; stty -echo; fi
read -r TYPED
if [ -t 0 ]; then stty echo; fi
echo
ENTROPY=$( { printf '%s' "$TYPED"; head -c 64 /dev/urandom; } | shasum -a 512 | cut -d' ' -f1)
TYPED=""
# The entropy goes to snarkjs's API on stdin, never through a command line,
# an environment variable or snarkjs's own prompt, which echoes its input when
# the output is a terminal. See contribute.mjs.
printf '%s' "$ENTROPY" | node contribute.mjs "$LAST" "$NEXT" "$NAME"
ENTROPY=""
echo
echo "Wrote $NEXT. Publish the contribution hash above."
