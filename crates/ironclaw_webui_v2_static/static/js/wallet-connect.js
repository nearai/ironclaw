import { NearConnector } from "@hot-labs/near-connect";

// Fixed NEP-413 login message NEAR AI's `/v1/auth/near` validates. Both values
// are checked server-side by NEAR AI, so they must match exactly. They live here
// because the wallet signs over them; the backend only relays.
const MESSAGE = "Sign in to NEAR AI Cloud";
const RECIPIENT = "cloud.near.ai";

const statusEl = document.getElementById("status");
function setStatus(text, isError) {
  statusEl.textContent = text;
  statusEl.classList.toggle("error", Boolean(isError));
}

// NEAR AI requires the first 8 nonce bytes to be the big-endian epoch-millis
// timestamp (validated within a 5-minute window); the remaining 24 are random.
function buildNonce() {
  const nonce = new Uint8Array(32);
  new DataView(nonce.buffer).setBigUint64(0, BigInt(Date.now()), false);
  crypto.getRandomValues(nonce.subarray(8));
  return nonce;
}

const channelName = new URLSearchParams(window.location.search).get("channel");

function postResult(payload) {
  if (!channelName || typeof BroadcastChannel !== "function") {
    return;
  }
  const channel = new BroadcastChannel(channelName);
  channel.postMessage(payload);
  channel.close();
}

async function run() {
  if (!channelName || typeof BroadcastChannel !== "function") {
    setStatus("Open this from the IronClaw app.", true);
    return;
  }
  try {
    const connector = new NearConnector({
      network: "mainnet",
      features: { signMessage: true },
    });
    setStatus("Choose a wallet to continue…");
    await connector.connect();
    const wallet = await connector.wallet();

    setStatus("Approve the signature in your wallet…");
    const nonce = buildNonce();
    const signed = await wallet.signMessage({
      message: MESSAGE,
      recipient: RECIPIENT,
      nonce,
    });

    // The backend rebuilds NEAR AI's request from these fields; `nonce` goes
    // back as a plain byte array (NEAR AI wants a 32-int JSON array, not base64).
    postResult({
      type: "nearai-wallet-login",
      ok: true,
      accountId: signed.accountId,
      publicKey: signed.publicKey,
      signature: signed.signature,
      message: MESSAGE,
      recipient: RECIPIENT,
      nonce: Array.from(nonce),
    });
    setStatus("Signed. You can close this window.");
    window.close();
  } catch (_err) {
    postResult({ type: "nearai-wallet-login", ok: false });
    setStatus("Wallet sign-in was cancelled or failed.", true);
  }
}

run();
