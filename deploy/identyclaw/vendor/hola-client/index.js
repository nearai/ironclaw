const { computeHolaChecksum, HOLA_CHECKSUM_ALPHABET } = require("./lib/checksum");
const {
  buildAndSign,
  buildCanonicalPrefix,
  encodeSignatureBase32,
  normalizeRecipient,
  normalizeTokenId,
  PROTOCOL_SUFFIX
} = require("./lib/sign");
const { getNonce } = require("./lib/nonce-api");
const { nearPrivateKeyToSigningSecretKey } = require("./lib/near-key");
const {
  generateNearImplicitAccount,
  validateNearCredentialsOutputDir,
  writeNearCredentialsFile,
  hasNearCredentialsDirSuffix
} = require("./lib/generate-near-account");
const { createHola } = require("./lib/create-hola");
const { parseHola } = require("./lib/parse");
const {
  COLLABORATION_SCHEMA,
  DEFAULT_MAX_AGE_MS,
  IDENTYCLAW_FENCE,
  buildCollaborationEnvelope,
  parseCollaborationEnvelope,
  extractIdentyclawFence,
  validateCollaborationEnvelope,
  assertCollaborationTrust,
  formatSessionsSendMessage
} = require("./lib/collaboration-envelope");

module.exports = {
  computeHolaChecksum,
  HOLA_CHECKSUM_ALPHABET,
  buildAndSign,
  buildCanonicalPrefix,
  encodeSignatureBase32,
  normalizeRecipient,
  normalizeTokenId,
  PROTOCOL_SUFFIX,
  getNonce,
  nearPrivateKeyToSigningSecretKey,
  generateNearImplicitAccount,
  validateNearCredentialsOutputDir,
  writeNearCredentialsFile,
  hasNearCredentialsDirSuffix,
  createHola,
  parseHola,
  COLLABORATION_SCHEMA,
  DEFAULT_MAX_AGE_MS,
  IDENTYCLAW_FENCE,
  buildCollaborationEnvelope,
  parseCollaborationEnvelope,
  extractIdentyclawFence,
  validateCollaborationEnvelope,
  assertCollaborationTrust,
  formatSessionsSendMessage,
  /** @deprecated use parseHola */
  verify: parseHola
};
