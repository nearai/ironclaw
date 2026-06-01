import { React } from "../../../lib/html.js";
import { fetchNearChallenge, verifyNear } from "../../../lib/api.js";

// `@hot-labs/near-connect` is loaded lazily from esm.sh the first time
// the user clicks the NEAR button, so the wallet SDK never enters the
// initial bundle. The module URL matches the v1 gateway's pinned
// version so both surfaces speak the same NEP-413 dialect.
const NEAR_CONNECT_MODULE = "https://esm.sh/@hot-labs/near-connect@0.11";

// Process-wide singleton: `NearConnector` keeps its own wallet-selector
// state, so re-instantiating it per click would drop a connected
// wallet. Keyed by network so a config flip (testnet ⇄ mainnet) builds
// a fresh connector instead of reusing a wrong-chain one.
let cachedConnector = null;
let cachedNetwork = null;

async function getConnector(network) {
  if (cachedConnector && cachedNetwork === network) return cachedConnector;
  const mod = await import(NEAR_CONNECT_MODULE);
  cachedConnector = new mod.NearConnector({ network });
  cachedNetwork = network;
  return cachedConnector;
}

// Decode the hex challenge nonce to the 32-byte Uint8Array the wallet
// binds into the NEP-413 payload. The server reconstructs the same
// bytes from the echoed hex on verify.
function hexToBytes(hex) {
  const pairs = hex.match(/.{2}/g) || [];
  return new Uint8Array(pairs.map((b) => parseInt(b, 16)));
}

/**
 * NEAR wallet login flow. Returns `{ status, error, signInWithNear }`.
 *
 * `status` is one of `idle | connecting | signing | verifying`. On a
 * successful verify the returned bearer is handed to `onToken`, which
 * the login page wires to `auth.signIn`.
 */
export function useNearLogin({ onToken }) {
  const [status, setStatus] = React.useState("idle");
  const [error, setError] = React.useState("");
  // Guard against overlapping clicks (double-tap, Enter+click): a flow
  // already in progress must not start a second wallet handshake.
  const inFlight = React.useRef(false);

  const signInWithNear = React.useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setError("");
    try {
      setStatus("connecting");
      const challenge = await fetchNearChallenge();

      const connector = await getConnector(challenge.network || "mainnet");
      const wallet = await connector.connect();
      const accounts = await wallet.getAccounts();
      if (!accounts || accounts.length === 0) {
        throw new Error("No NEAR account found in the connected wallet.");
      }
      const accountId = accounts[0].accountId;

      setStatus("signing");
      const signed = await wallet.signMessage({
        message: challenge.message,
        recipient: challenge.recipient || "ironclaw",
        nonce: hexToBytes(challenge.nonce),
      });

      setStatus("verifying");
      const token = await verifyNear({
        accountId,
        publicKey: signed.publicKey,
        signature: signed.signature,
        nonce: challenge.nonce,
      });

      // Success: hand the bearer to the page. Leave `status` at
      // `verifying` — the page navigates away on sign-in, so resetting
      // to `idle` would only flash the default label first.
      onToken(token);
    } catch (err) {
      setStatus("idle");
      setError(err?.message || "NEAR wallet sign-in failed.");
    } finally {
      inFlight.current = false;
    }
  }, [onToken]);

  return { status, error, signInWithNear };
}
