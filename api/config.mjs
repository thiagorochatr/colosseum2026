// The public configuration the web app needs: addresses, RPCs, fees, and the
// relayer's and relay's addresses, which the proof's binding names.

import { TEMPO, SOLANA, reply } from "./_lib/common.mjs";
import { relayerAddress } from "./_lib/tempo.mjs";
import { relayAddress } from "./_lib/solana.mjs";

export default function handler(req, res) {
  try {
    // The browser gets only its own RPC endpoint, never the functions' key.
    const { initialBlocklist: _, rpc: _server, browserRpc, ...solana } = SOLANA;
    // Each chain on its own: a key missing for one must not take down the other's page.
    const part = (build) => {
      try {
        return build();
      } catch (e) {
        return { error: e.message };
      }
    };
    reply(res, 200, {
      tempo: part(() => (({ rpc: _server, browserRpc: rpc, ...tempo }) => ({ ...tempo, rpc, relayer: relayerAddress() }))(TEMPO)),
      solana: part(() => ({ ...solana, rpc: browserRpc, relay: relayAddress() })),
    });
  } catch (e) {
    reply(res, 500, { error: e.message });
  }
}
