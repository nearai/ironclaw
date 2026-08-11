const { getNonce } = require("./nonce-api");
const { buildAndSign, normalizeRecipient, normalizeTokenId } = require("./sign");
const { nearPrivateKeyToSigningSecretKey } = require("./near-key");

/**
 * Fetch a fresh nonce and build a signed standard-format HOLA line locally.
 * Private key stays on the caller machine — only JWT is sent to the API for nonce fetch.
 *
 * @param {object} params
 * @param {string} params.nearPrivateKey - NEAR ed25519 private key (ed25519:...)
 * @param {string} params.jwt - Bearer JWT from POST /api/login
 * @param {string} params.tokenId - 12-letter Passport ID of the signer
 * @param {string} [params.baseUrl] - API base URL
 * @param {string} [params.recipient] - HOLA recipient (default MUNDO)
 * @returns {Promise<object>}
 */
async function createHola({
  nearPrivateKey,
  jwt,
  tokenId,
  baseUrl = "https://api.identyclaw.com",
  recipient = "MUNDO"
}) {
  if (!tokenId || typeof tokenId !== "string") {
    throw new Error("tokenId is required");
  }

  const nonce = await getNonce({ baseUrl, jwt });
  const privateKey = nearPrivateKeyToSigningSecretKey(nearPrivateKey);
  const signed = buildAndSign({
    recipient,
    tokenId,
    timestamp: nonce.timestamp,
    noncetsHex: nonce.noncetsHex,
    privateKey
  });

  return {
    hola: signed.hola,
    noncetsHex: nonce.noncetsHex,
    timestamp: nonce.timestamp,
    tokenId: normalizeTokenId(tokenId),
    recipient: normalizeRecipient(recipient),
    signatureB32: signed.signatureB32,
    checksum: signed.checksum,
    requestId: nonce.requestId
  };
}

module.exports = {
  createHola
};
