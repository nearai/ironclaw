const { randomUUID } = require("crypto");
const { parseHola } = require("./parse");
const { normalizeTokenId } = require("./sign");

const COLLABORATION_SCHEMA = "identyclaw.collaboration.v1";
const DEFAULT_MAX_AGE_MS = 5 * 60 * 1000;
const IDENTYCLAW_FENCE = "```identyclaw";

/**
 * @param {object} params
 * @param {string} params.fromTokenId
 * @param {string} params.hola
 * @param {string} params.taskType
 * @param {object} params.taskPayload
 * @param {string} [params.toTokenId]
 * @param {string} [params.toContactUri]
 * @param {string} [params.messageId]
 * @param {string} [params.timestamp]
 * @param {object} [params.channelHints]
 */
function buildCollaborationEnvelope({
  fromTokenId,
  hola,
  taskType,
  taskPayload,
  toTokenId,
  toContactUri,
  messageId,
  timestamp,
  channelHints
}) {
  if (!fromTokenId || !hola || !taskType || taskPayload == null) {
    throw new Error("fromTokenId, hola, taskType, and taskPayload are required");
  }

  const envelope = {
    schema: COLLABORATION_SCHEMA,
    messageId: messageId || randomUUID(),
    timestamp: timestamp || new Date().toISOString(),
    from: { tokenId: normalizeTokenId(fromTokenId) },
    hola: String(hola).trim(),
    task: {
      type: String(taskType),
      payload: taskPayload
    }
  };

  if (toTokenId) {
    envelope.to = { tokenId: normalizeTokenId(toTokenId) };
    if (toContactUri) {
      envelope.to.contactUri = String(toContactUri);
    }
  } else if (toContactUri) {
    envelope.to = { contactUri: String(toContactUri) };
  }

  if (channelHints && typeof channelHints === "object") {
    envelope.channelHints = channelHints;
  }

  return envelope;
}

/**
 * @param {string|object} input
 * @returns {object}
 */
function parseCollaborationEnvelope(input) {
  let parsed = input;

  if (typeof parsed === "string") {
    const trimmed = parsed.trim();
    const fenced = extractIdentyclawFence(trimmed);
    parsed = JSON.parse(fenced || trimmed);
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error("envelope must be a JSON object");
  }

  return parsed;
}

/**
 * @param {string} message
 * @returns {string|null}
 */
function extractIdentyclawFence(message) {
  if (typeof message !== "string" || message.length === 0) {
    return null;
  }

  const start = message.indexOf(IDENTYCLAW_FENCE);
  if (start === -1) {
    return null;
  }

  const bodyStart = start + IDENTYCLAW_FENCE.length;
  const end = message.indexOf("```", bodyStart);
  if (end === -1) {
    return null;
  }

  return message.slice(bodyStart, end).trim();
}

/**
 * @param {object} envelope
 * @param {number} [maxAgeMs]
 * @returns {{ ok: true, envelope: object } | { ok: false, reason: string, step: string }}
 */
function validateCollaborationEnvelope(envelope, maxAgeMs = DEFAULT_MAX_AGE_MS) {
  if (envelope.schema !== COLLABORATION_SCHEMA) {
    return { ok: false, step: "parse", reason: `expected schema ${COLLABORATION_SCHEMA}` };
  }

  if (!envelope.messageId || !envelope.timestamp || !envelope.from?.tokenId || !envelope.hola || !envelope.task?.type) {
    return { ok: false, step: "parse", reason: "missing required envelope fields" };
  }

  const ageMs = Date.now() - Date.parse(envelope.timestamp);
  if (!Number.isFinite(ageMs)) {
    return { ok: false, step: "freshness", reason: "invalid envelope timestamp" };
  }
  if (ageMs < 0) {
    return { ok: false, step: "freshness", reason: "envelope timestamp is in the future" };
  }
  if (ageMs > maxAgeMs) {
    return { ok: false, step: "freshness", reason: `envelope older than ${maxAgeMs}ms` };
  }

  const holaShape = parseHola(envelope.hola);
  if (!holaShape.valid) {
    return { ok: false, step: "hola-shape", reason: holaShape.reason || "invalid HOLA format" };
  }

  return { ok: true, envelope, holaShape };
}

/**
 * Pure trust decision after POST /api/identity/verify (and optional isauthorizedsigner).
 *
 * @param {object} envelope
 * @param {object} verifyResult - API verify response with verified + peerTokenId
 * @param {{ authorized?: boolean }|null} [signerResult]
 * @returns {{ ok: true, peerTokenId: string } | { ok: false, reason: string, step: string }}
 */
function assertCollaborationTrust(envelope, verifyResult, signerResult = null) {
  const shape = validateCollaborationEnvelope(envelope);
  if (!shape.ok) {
    return shape;
  }

  if (!verifyResult?.verified) {
    return { ok: false, step: "verify", reason: verifyResult?.reason || "HOLA not verified" };
  }

  const peerTokenId = normalizeTokenId(verifyResult.peerTokenId);
  const declaredTokenId = normalizeTokenId(envelope.from.tokenId);

  if (peerTokenId !== declaredTokenId) {
    return {
      ok: false,
      step: "identity-match",
      reason: `peerTokenId ${peerTokenId} does not match envelope.from.tokenId ${declaredTokenId}`
    };
  }

  const holaShape = parseHola(envelope.hola);
  if (holaShape.isSubagentFormat) {
    if (!signerResult?.authorized) {
      return { ok: false, step: "delegation", reason: "subagent HOLA requires authorized signer" };
    }
  }

  return { ok: true, peerTokenId };
}

/**
 * @param {object} envelope
 * @param {string} [summaryLine]
 * @returns {string}
 */
function formatSessionsSendMessage(envelope, summaryLine) {
  const intro =
    summaryLine ||
    `Trusted A2A message (${envelope.task.type}). Verify the identyclaw block before acting.`;
  return `${intro}\n\n${IDENTYCLAW_FENCE}\n${JSON.stringify(envelope, null, 2)}\n\`\`\``;
}

module.exports = {
  COLLABORATION_SCHEMA,
  DEFAULT_MAX_AGE_MS,
  IDENTYCLAW_FENCE,
  buildCollaborationEnvelope,
  parseCollaborationEnvelope,
  extractIdentyclawFence,
  validateCollaborationEnvelope,
  assertCollaborationTrust,
  formatSessionsSendMessage
};
