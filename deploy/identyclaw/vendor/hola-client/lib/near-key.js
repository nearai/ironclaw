const bs58 = require("bs58");
const nacl = require("tweetnacl");

/**
 * NEAR `ed25519:...` private key → 64-byte tweetnacl Ed25519 secret key for signing.
 * Never send this value to the IdentyClaw API — use only on the agent host.
 *
 * @param {string} nearPrivateKey
 * @returns {Uint8Array}
 */
function nearPrivateKeyToSigningSecretKey(nearPrivateKey) {
  if (typeof nearPrivateKey !== "string" || nearPrivateKey.trim().length === 0) {
    throw new Error("nearPrivateKey must be a non-empty string");
  }

  const keyBody = nearPrivateKey.replace(/^ed25519:/, "").trim();
  const decoded = bs58.decode(keyBody);

  if (decoded.length >= 64) {
    return decoded.slice(0, 64);
  }
  if (decoded.length >= 32) {
    return nacl.sign.keyPair.fromSeed(decoded.slice(0, 32)).secretKey;
  }

  throw new Error("Invalid NEAR private key: decoded length is less than 32 bytes");
}

module.exports = {
  nearPrivateKeyToSigningSecretKey
};
