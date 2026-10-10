// The public configuration the web app needs: addresses, RPCs, fees, and the
// relayer's and relay's addresses, which the proof's binding names.

import { TEMPO, SOLANA, reply } from "./_lib/common.mjs";
import { relayerAddress } from "./_lib/tempo.mjs";
import { relayAddress } from "./_lib/solana.mjs";

export default function handler(req, res) {
  try {
    const { initialBlocklist: _, ...solana } = SOLANA;
    reply(res, 200, { tempo: { ...TEMPO, relayer: relayerAddress() }, solana: { ...solana, relay: relayAddress() } });
  } catch (e) {
    reply(res, 500, { error: e.message });
  }
}
