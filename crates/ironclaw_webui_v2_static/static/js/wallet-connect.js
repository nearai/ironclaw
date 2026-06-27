import { NearConnector } from "@hot-labs/near-connect";
import {
  NEAR_AI_WALLET_LOGIN_MESSAGE,
  NEAR_AI_WALLET_LOGIN_RECIPIENT,
  buildNearAiWalletLoginNonce,
  nearAiWalletLoginFailurePayload,
  nearAiWalletLoginSuccessPayload,
} from "./lib/wallet-connect-core.js";

// Fixed NEP-413 login message NEAR AI's `/v1/auth/near` validates. Both values
// are checked server-side by NEAR AI, so they must match exactly. They live here
// because the wallet signs over them; the backend only relays.
const MESSAGE = NEAR_AI_WALLET_LOGIN_MESSAGE;
const RECIPIENT = NEAR_AI_WALLET_LOGIN_RECIPIENT;

const statusEl = document.getElementById("status");
function setStatus(text, isError) {
  statusEl.textContent = text;
  statusEl.classList.toggle("error", Boolean(isError));
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
    const nonce = buildNearAiWalletLoginNonce();
    const signed = await wallet.signMessage({
      message: MESSAGE,
      recipient: RECIPIENT,
      nonce,
    });

    // The backend rebuilds NEAR AI's request from these fields; `nonce` goes
    // back as a plain byte array (NEAR AI wants a 32-int JSON array, not base64).
    postResult(nearAiWalletLoginSuccessPayload(signed, nonce));
    setStatus("Signed. You can close this window.");
    window.close();
  } catch (_err) {
    postResult(nearAiWalletLoginFailurePayload());
    setStatus("Wallet sign-in was cancelled or failed.", true);
  }
}

run();
