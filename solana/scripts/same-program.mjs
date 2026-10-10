// Exits 0 when the dumped program data is the built program followed only by
// zero padding, and says where they differ otherwise.
//
//   node scripts/same-program.mjs <built.so> <dumped.so>
import { readFileSync } from "node:fs";

const [builtPath, dumpedPath] = process.argv.slice(2);
const built = readFileSync(builtPath);
const dumped = readFileSync(dumpedPath);

let problem = null;
if (dumped.length < built.length) problem = `deployed data is shorter (${dumped.length} < ${built.length} bytes)`;
else {
  const at = built.findIndex((b, i) => b !== dumped[i]);
  if (at !== -1) problem = `first difference at byte ${at}`;
  else if (dumped.subarray(built.length).some((b) => b !== 0)) problem = "non-zero bytes after the program";
}
if (problem) {
  console.error(`${builtPath}: ${problem}`);
  process.exit(1);
}
