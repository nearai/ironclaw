const base32 = require("hi-base32");
const nacl = require("tweetnacl");
const { computeHolaChecksum } = require("./checksum");

const PROTOCOL_SUFFIX = "API.IDENTYCLAW.COM/";

function encodeSignatureBase32(signatureBytes) {
  return base32.encode(Buffer.from(signatureBytes)).replace(/=+$/g, "").toUpperCase();
}

function normalizeRecipient(recipient) {
  const value = recipient && String(recipient).trim().length > 0 ? recipient : "MUNDO";
  return String(value).toUpperCase();
}

function normalizeTokenId(tokenId) {
  return String(tokenId || "").toLowerCase();
}

/**
 * Build the unsigned canonical HOLA prefix (uppercase, trailing slash before signature).
 *
 * @param {object} params
 * @param {string} params.recipient
 * @param {string} params.tokenId - 12-letter passport ID (standard format)
 * @param {string} params.timestamp - ISO-8601 from GET /api/holanonce16ts
 * @param {string} params.noncetsHex - 32 hex chars from GET /api/holanonce16ts
 * @param {string} [params.delegateId] - subagent format
 * @param {string} [params.issuerTokenId] - subagent format
 * @param {string} [params.subagentPublicKey] - base32 public key (subagent format)
 * @returns {string}
 */
function buildCanonicalPrefix({
  recipient,
  tokenId,
  timestamp,
  noncetsHex,
  delegateId,
  issuerTokenId,
  subagentPublicKey
}) {
  const normalizedRecipient = normalizeRecipient(recipient);
  const normalizedNonce = String(noncetsHex || "").toUpperCase();
  const normalizedTimestamp = String(timestamp || "");

  if (delegateId && issuerTokenId && subagentPublicKey) {
    const message =
      `HOLA/${normalizedRecipient}/${String(delegateId).toUpperCase()}/` +
      `${normalizeTokenId(issuerTokenId)}/${String(subagentPublicKey).toUpperCase()}/` +
      `${normalizedTimestamp}/${normalizedNonce}/${PROTOCOL_SUFFIX}`;
    return message.toUpperCase();
  }

  const message =
    `HOLA/${normalizedRecipient}/${normalizeTokenId(tokenId)}/` +
    `${normalizedTimestamp}/${normalizedNonce}/${PROTOCOL_SUFFIX}`;
  return message.toUpperCase();
}

/**
 * Sign a HOLA line. privateKey must be a 64-byte Ed25519 secret key (tweetnacl format).
 *
 * @param {object} params
 * @param {string} params.recipient
 * @param {string} params.tokenId
 * @param {string} params.timestamp
 * @param {string} params.noncetsHex
 * @param {Uint8Array} params.privateKey
 * @param {string} [params.delegateId]
 * @param {string} [params.issuerTokenId]
 * @param {string} [params.subagentPublicKey]
 * @returns {{ hola: string, canonicalPrefix: string, signatureB32: string, checksum: string }}
 */
function buildAndSign(params) {
  const { privateKey } = params;
  if (!(privateKey instanceof Uint8Array) || privateKey.length !== 64) {
    throw new Error("privateKey must be a 64-byte Ed25519 secret key (tweetnacl format)");
  }

  const canonicalPrefix = buildCanonicalPrefix(params);
  const messageBytes = new TextEncoder().encode(canonicalPrefix);
  const signature = nacl.sign.detached(messageBytes, privateKey);
  const signatureB32 = encodeSignatureBase32(signature);
  const checksumPrefix = `${canonicalPrefix}${signatureB32}/`;
  const checksum = computeHolaChecksum(checksumPrefix);
  const hola = `${canonicalPrefix}${signatureB32}/${checksum}`;

  return {
    hola,
    canonicalPrefix,
    signatureB32,
    checksum
  };
}

module.exports = {
  PROTOCOL_SUFFIX,
  encodeSignatureBase32,
  buildCanonicalPrefix,
  buildAndSign,
  normalizeRecipient,
  normalizeTokenId
};
