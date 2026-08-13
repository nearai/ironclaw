const { computeHolaChecksum } = require("./checksum");

/**
 * Parse and validate HOLA format + checksum (no signature or on-chain checks).
 *
 * @param {string} hola
 * @returns {{ valid: boolean, reason?: string, recipient?: string, tokenId?: string, isSubagentFormat?: boolean }}
 */
function parseHola(hola) {
  if (typeof hola !== "string" || hola.trim().length === 0) {
    return { valid: false, reason: "hola must be a non-empty string" };
  }

  const trimmed = hola.trim();
  if (!/^HOLA\//i.test(trimmed)) {
    return { valid: false, reason: "expected HOLA/ prefix" };
  }

  const parts = trimmed.split("/");
  if (parts.length !== 8 && parts.length !== 10) {
    return { valid: false, reason: "invalid HOLA segment count" };
  }

  const checksumChar = parts[parts.length - 1];
  const signatureB32 = parts[parts.length - 2];
  const protocolPart = parts[parts.length - 3];
  const canonicalPrefix = `${parts.slice(0, parts.length - 2).join("/")}/`.toUpperCase();
  const checksumPrefix = `${canonicalPrefix}${signatureB32.toUpperCase()}/`;
  const expectedChecksum = computeHolaChecksum(checksumPrefix);

  if (checksumChar.toUpperCase() !== expectedChecksum) {
    return { valid: false, reason: "checksum mismatch" };
  }

  if (protocolPart.toUpperCase() !== "API.IDENTYCLAW.COM") {
    return { valid: false, reason: "unsupported protocol host" };
  }

  const isSubagentFormat = parts.length === 10;
  const recipient = parts[1];

  if (isSubagentFormat) {
    return {
      valid: true,
      recipient,
      delegateId: parts[2],
      issuerTokenId: parts[3],
      subagentPublicKey: parts[4],
      timestamp: parts[5],
      noncetsHex: parts[6],
      signatureB32,
      checksum: checksumChar,
      isSubagentFormat: true
    };
  }

  return {
    valid: true,
    recipient,
    tokenId: parts[2],
    timestamp: parts[3],
    noncetsHex: parts[4],
    signatureB32,
    checksum: checksumChar,
    isSubagentFormat: false
  };
}

module.exports = {
  parseHola
};
