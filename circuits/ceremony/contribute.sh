#!/bin/sh
# Adds one phase-2 contribution on top of the latest key in this directory.
#
#   sh ceremony/contribute.sh "Your name"
#
# Run it in your own terminal. It asks for some random text, mixes it with 64
# bytes from /dev/urandom, and hands the result to snarkjs. The entropy is never
# printed, never written to disk, and never passed as an argument or to a prompt. Your contribution is safe as long as that
# entropy is gone when the script exits.
set -eu
cd "$(dirname "$0")"
NAME=${1:?usage: contribute.sh "Your name"}
LAST=$(ls withdraw_[0-9][0-9][0-9][0-9].zkey | sort | tail -1)
N=$(echo "$LAST" | sed 's/withdraw_\([0-9]*\)\.zkey/\1/')
NEXT=$(printf "withdraw_%04d.zkey" "$(expr "$N" + 1)")
printf "Type some random text, then Enter (it is not shown): "
if [ -t 0 ]; then stty -echo; fi; read -r TYPED; if [ -t 0 ]; then stty echo; fi; echo
ENTROPY=$( { printf '%s' "$TYPED"; head -c 64 /dev/urandom; } | shasum -a 512 | cut -d' ' -f1)
TYPED=""
# The entropy goes to snarkjs's API on stdin, never through a command line,
# an environment variable or snarkjs's own prompt, which echoes its input when
# the output is a terminal. See contribute.mjs.
printf '%s' "$ENTROPY" | node contribute.mjs "$LAST" "$NEXT" "$NAME"
ENTROPY=""
echo
echo "Wrote $NEXT. Publish the contribution hash above."
