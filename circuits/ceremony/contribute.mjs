// Adds one phase-2 contribution through snarkjs's API. Called by contribute.sh.
//
//   <entropy on stdin> | node contribute.mjs <old.zkey> <new.zkey> <name>
//
// The entropy is read from stdin into memory and passed straight to snarkjs. It
// never goes through a command line, an environment variable or snarkjs's
// interactive prompt, which echoes its input when the output is a terminal.

import * as snarkjs from "snarkjs";

const [oldZkey, newZkey, name] = process.argv.slice(2);
if (!oldZkey || !newZkey || !name) throw new Error("usage: contribute.mjs <old.zkey> <new.zkey> <name>");

let entropy = "";
for await (const chunk of process.stdin) entropy += chunk;
entropy = entropy.trim();
if (entropy.length < 64) throw new Error("expected at least 64 characters of entropy on stdin");

// Defence in depth: nothing snarkjs logs may contain the entropy.
const safe = (write) => (...args) => {
  const line = args.join(" ");
  if (line.includes(entropy)) throw new Error("refusing to print a line that contains the entropy");
  write(line);
};
const logger = {
  info: safe(console.log),
  warn: safe(console.warn),
  error: safe(console.error),
  debug: () => {},
};

await snarkjs.zKey.contribute(oldZkey, newZkey, name, entropy, logger);
entropy = "";
await globalThis.curve_bn128?.terminate();
