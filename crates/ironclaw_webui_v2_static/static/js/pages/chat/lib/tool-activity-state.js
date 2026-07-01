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
  upsertToolActivityMessage(setMessages, card, stateRef);
}

export function failGateToolActivity(
  setMessages,
  gate,
  stateRef,
  toolError = "gate_declined",
) {
  const card = toolCardFromGate(gate, {
    toolStatus: "declined",
    toolError,
    toolErrorKind: "gate_declined",
  });
  if (!card) return;
  upsertToolActivityMessage(setMessages, card, stateRef);
}

export function upsertToolActivityMessage(
  setMessages,
  card,
  stateRef,
) {
  if (!card) return;
  let incoming = normalizeToolCard(card);
  incoming = applyRememberedTerminal(incoming, stateRef);
  setMessages((prev) => {
    const targetId = toolMessageId(incoming);
    const existing = findToolActivityIndex(prev, incoming, targetId);
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
  const isGatePrompt = gate?.kind === "gate";
  const isAuthPrompt = gate?.kind === "auth_required";
  const shouldFallbackToGateIdentity =
    isAuthPrompt && overrides.toolStatus === "declined";
  if (
    !gate?.runId ||
    !gate?.gateRef ||
    (!gate.invocationId && !shouldFallbackToGateIdentity) ||
    (!isGatePrompt && !isAuthPrompt)
  ) {
    return null;
  }
  const invocationId = gate.invocationId || fallbackGateInvocationId(gate);
  const displaySource = gate.toolName || gate.headline || gate.gateKind || "gate";
  return {
    invocationId,
    callId: invocationId,
    capabilityId: gate.toolName || gate.gateKind || null,
    toolName: toolDisplayName(displaySource) || displaySource,
    toolStatus: overrides.toolStatus || "running",
    toolDetail: null,
    toolParameters: null,
    toolResultPreview: null,
    toolError: overrides.toolError || null,
    toolErrorKind: overrides.toolErrorKind || null,
    toolErrorIsBareKind: !!overrides.toolErrorKind && !overrides.toolError,
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

function fallbackGateInvocationId(gate) {
  return `gate:${gate.runId}:${gate.kind}:${gate.gateRef}`;
}

function toolMessageId(card) {
  return `tool-${card.invocationId}`;
}

function findToolActivityIndex(messages, card, targetId) {
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

  return -1;
}

function mergeToolActivity(current, incoming) {
  const currentTerminal = isTerminalToolStatus(current.toolStatus);
  const incomingTerminal = isTerminalToolStatus(incoming.toolStatus);
  const keepCurrentTerminal = currentTerminal && !incomingTerminal;
  const incomingGateOnly = incoming.gateActivity && !current.gateActivity;
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
    toolName: incomingGateOnly
      ? current.toolName
      : incoming.toolName || current.toolName,
    toolStatus: keepCurrentTerminal ? current.toolStatus : incoming.toolStatus,
    toolError: mergedToolError(current, incoming),
    toolErrorKind: incoming.toolErrorKind || current.toolErrorKind || null,
    toolErrorIsBareKind: mergedToolErrorIsBareKind(current, incoming),
    updatedAt: keepCurrentTerminal
      ? current.updatedAt || incoming.updatedAt
      : incoming.updatedAt || current.updatedAt,
    turnRunId: incoming.turnRunId || current.turnRunId || null,
    gateRef: incoming.gateRef || current.gateRef || null,
    gateActivity: current.gateActivity && incoming.gateActivity,
    capabilityId: incomingGateOnly
      ? current.capabilityId || incoming.capabilityId || null
      : incoming.capabilityId || current.capabilityId || null,
    activityOrder: mergedActivityOrder(current, incoming),
    activityOrderSource: incoming.activityOrderSource || current.activityOrderSource || null,
  };
  if (current.gateActivity && !incoming.gateActivity) {
    merged.id = toolMessageId(incoming);
    merged.gateActivity = false;
  }
  return merged;
}

// A capability failure surfaces its detail on two frames that race: the
// runtime-payload `capability_activity` deliberately carries no `error_detail`
// (its `toolError` is just the bare kind, e.g. "invalid_input"), while the
// `capability_display_preview` carries the real reason. A plain
// `incoming || current` lets a late bare-kind activity frame clobber a detail
// already set by the preview. Never downgrade a richer error to the bare kind:
// if the incoming error is only the kind string and the current one is a
// different (detailed) message, keep the current one.
function mergedToolError(current, incoming) {
  const incomingError = incoming.toolError || null;
  const currentError = current.toolError || null;
  if (!incomingError) return currentError;
  if (incoming.toolErrorIsBareKind && currentError && currentError !== incomingError) {
    return currentError;
  }
  return incomingError;
}

function mergedToolErrorIsBareKind(current, incoming) {
  const incomingError = incoming.toolError || null;
  const currentError = current.toolError || null;
  if (incoming.toolErrorIsBareKind && currentError && currentError !== incomingError) {
    return !!current.toolErrorIsBareKind;
  }
  if (incomingError) return !!incoming.toolErrorIsBareKind;
  return !!current.toolErrorIsBareKind;
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

function normalizeToolCard(card) {
  const normalizedName = toolDisplayName(card.toolName || card.capabilityId);
  return {
    ...card,
    toolName: normalizedName || card.toolName || "tool",
    toolErrorIsBareKind: Boolean(card.toolErrorIsBareKind),
  };
}
