export const NEAR_AI_WALLET_LOGIN_MESSAGE = "Sign in to NEAR AI Cloud";
export const NEAR_AI_WALLET_LOGIN_RECIPIENT = "cloud.near.ai";
export const NEAR_AI_WALLET_LOGIN_EVENT = "nearai-wallet-login";

export function buildNearAiWalletLoginNonce({
  now = Date.now(),
  getRandomValues = (target) => crypto.getRandomValues(target),
} = {}) {
  const nonce = new Uint8Array(32);
  new DataView(nonce.buffer).setBigUint64(0, BigInt(now), false);
  getRandomValues(nonce.subarray(8));
  return nonce;
}

export function nearAiWalletLoginSuccessPayload(signed, nonce) {
  return {
    type: NEAR_AI_WALLET_LOGIN_EVENT,
    ok: true,
    accountId: signed.accountId,
    publicKey: signed.publicKey,
    signature: signed.signature,
    message: NEAR_AI_WALLET_LOGIN_MESSAGE,
    recipient: NEAR_AI_WALLET_LOGIN_RECIPIENT,
    nonce: Array.from(nonce),
  };
}

export function nearAiWalletLoginFailurePayload() {
  return { type: NEAR_AI_WALLET_LOGIN_EVENT, ok: false };
}
