import {
  isTerminalToolStatus,
  toolDisplayName,
} from "./history-messages.js";

export function createToolActivityState() {
  return {
    terminalByInvocation: new Map(),
  };
}

export function resetToolActivityState(stateRef) {
  stateRef?.current?.terminalByInvocation?.clear();
}

export function ensureGateToolActivity(setMessages, gate, stateRef) {
  const card = toolCardFromGate(gate, { toolStatus: "running" });
  if (!card) return;
  upsertToolActivityMessage(setMessages, card, stateRef, {
    matchGate: true,
  });
}

export function failGateToolActivity(
  setMessages,
  gate,
  stateRef,
  toolError = "authorization",
) {
  const card = toolCardFromGate(gate, {
    toolStatus: "error",
    toolError,
  });
  if (!card) return;
  upsertToolActivityMessage(setMessages, card, stateRef, {
    matchGate: true,
  });
}

export function upsertToolActivityMessage(
  setMessages,
  card,
  stateRef,
  options = {},
) {
  if (!card) return;
  let incoming = normalizeToolCard(card);
  incoming = applyRememberedTerminal(incoming, stateRef);
  setMessages((prev) => {
    const targetId = toolMessageId(incoming);
    const existing = findToolActivityIndex(prev, incoming, targetId, options);
    if (existing >= 0) {
      const copy = [...prev];
      copy[existing] = mergeToolActivity(copy[existing], incoming);
      rememberTerminal(copy[existing], stateRef);
      return copy;
    }
    const message = {
      id: targetId,
      role: "tool_activity",
      ...incoming,
    };
    rememberTerminal(message, stateRef);
    return [...prev, message];
  });
}

function toolCardFromGate(gate, overrides = {}) {
  if (!gate?.runId || !gate?.gateRef || gate.kind !== "gate" || !gate.toolName) {
    return null;
  }
  const invocationId = gate.invocationId || `gate:${gate.runId}:${gate.gateRef}`;
  return {
    invocationId,
    callId: invocationId,
    capabilityId: gate.toolName,
    toolName: toolDisplayName(gate.toolName) || gate.toolName,
    toolStatus: overrides.toolStatus || "running",
    toolDetail: null,
    toolParameters: null,
    toolResultPreview: null,
    toolError: overrides.toolError || null,
    toolDurationMs: null,
    updatedAt: overrides.updatedAt || new Date().toISOString(),
    resultRef: null,
    truncated: false,
    outputBytes: null,
    outputKind: null,
    turnRunId: gate.runId,
    gateRef: gate.gateRef,
    gateActivity: true,
  };
}

function toolMessageId(card) {
  return `tool-${card.invocationId}`;
}

function findToolActivityIndex(messages, card, targetId, options) {
  const exact = messages.findIndex((message) => message?.id === targetId);
  if (exact >= 0) return exact;

  const gateRef = card.gateRef || null;
  if (gateRef) {
    const byGate = messages.findIndex(
      (message) =>
        message?.role === "tool_activity" &&
        message.turnRunId === card.turnRunId &&
        message.gateRef === gateRef,
    );
    if (byGate >= 0) return byGate;
  }

  if (!options.matchGate && !card.gateActivity) {
    const synthetic = messages.findIndex((message) =>
      canRealActivityAdoptSyntheticGate(message, card),
    );
    if (synthetic >= 0) return synthetic;
  }

  if (options.matchGate || card.gateActivity) {
    const byTool = messages.findIndex(
      (message) =>
        message?.role === "tool_activity" &&
        !message.gateRef &&
        message.gateActivity !== true &&
        !isTerminalToolStatus(message.toolStatus) &&
        message.turnRunId === card.turnRunId &&
        sameToolName(message.toolName, card.toolName),
    );
    if (byTool >= 0) return byTool;
  }

  return -1;
}

function canRealActivityAdoptSyntheticGate(message, card) {
  return (
    message?.role === "tool_activity" &&
    message.gateActivity === true &&
    message.turnRunId === card.turnRunId &&
    sameToolName(message.toolName, card.toolName)
  );
}

function mergeToolActivity(current, incoming) {
  const currentTerminal = isTerminalToolStatus(current.toolStatus);
  const incomingTerminal = isTerminalToolStatus(incoming.toolStatus);
  const keepCurrentTerminal = currentTerminal && !incomingTerminal;
  const merged = {
    ...current,
    ...incoming,
    id: current.id,
    role: "tool_activity",
    invocationId:
      current.gateActivity && !incoming.gateActivity
        ? incoming.invocationId
        : current.invocationId || incoming.invocationId,
    callId:
      current.gateActivity && !incoming.gateActivity
        ? incoming.callId
        : current.callId || incoming.callId,
    toolName: incoming.toolName || current.toolName,
    toolStatus: keepCurrentTerminal ? current.toolStatus : incoming.toolStatus,
    toolError: incoming.toolError || current.toolError,
    updatedAt: keepCurrentTerminal
      ? current.updatedAt || incoming.updatedAt
      : incoming.updatedAt || current.updatedAt,
    turnRunId: incoming.turnRunId || current.turnRunId || null,
    gateRef: incoming.gateRef || current.gateRef || null,
    gateActivity: current.gateActivity && incoming.gateActivity,
    capabilityId: incoming.capabilityId || current.capabilityId || null,
    activityOrder: mergedActivityOrder(current, incoming),
    activityOrderSource: incoming.activityOrderSource || current.activityOrderSource || null,
  };
  if (current.gateActivity && !incoming.gateActivity) {
    merged.id = toolMessageId(incoming);
    merged.gateActivity = false;
  }
  return merged;
}

function mergedActivityOrder(current, incoming) {
  return Number.isFinite(incoming.activityOrder)
    ? incoming.activityOrder
    : current.activityOrder;
}

function applyRememberedTerminal(card, stateRef) {
  if (!card?.invocationId) return card;
  if (isTerminalToolStatus(card.toolStatus)) {
    rememberTerminal(card, stateRef);
    return card;
  }
  const remembered = stateRef?.current?.terminalByInvocation?.get(card.invocationId);
  if (!remembered) return card;
  if (!Number.isFinite(card.activityOrder)) return remembered;
  return {
    ...remembered,
    activityOrder: card.activityOrder,
    activityOrderSource: card.activityOrderSource || remembered.activityOrderSource || null,
  };
}

function rememberTerminal(card, stateRef) {
  if (!card?.invocationId || !isTerminalToolStatus(card.toolStatus)) return;
  stateRef?.current?.terminalByInvocation?.set(card.invocationId, card);
}

function sameToolName(left, right) {
  if (!left || !right) return false;
  return toolDisplayName(left) === toolDisplayName(right);
}

function normalizeToolCard(card) {
  const normalizedName = toolDisplayName(card.toolName || card.capabilityId);
  return {
    ...card,
    toolName: normalizedName || card.toolName || "tool",
  };
}
