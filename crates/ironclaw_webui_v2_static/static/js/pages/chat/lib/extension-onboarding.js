const EXTENSION_ACTIVATE_CAPABILITY_ID = "builtin.extension_activate";
const SLACK_CONNECTED_CONTINUATION = "Slack is connected. Continue the previous request.";

export function onboardingFromExtensionActivatePreview(
  preview,
  currentThreadId,
  dismissedSourceIds,
) {
  if (!preview || preview.capability_id !== EXTENSION_ACTIVATE_CAPABILITY_ID) {
    return null;
  }
  const previewThreadId = preview.thread_id || null;
  if (previewThreadId && currentThreadId && previewThreadId !== currentThreadId) {
    return null;
  }
  const sourceMessageId = preview.invocation_id
    ? `tool-${preview.invocation_id}`
    : null;
  if (sourceMessageId && dismissedSourceIds?.has?.(sourceMessageId)) {
    return null;
  }
  return onboardingFromExtensionActivateOutput(
    parseJsonObject(preview.output_preview),
    previewThreadId || currentThreadId || null,
    sourceMessageId,
  );
}

export function onboardingFromToolMessages(
  messages,
  currentThreadId,
  dismissedSourceIds,
) {
  let sawSlackContinuation = false;
  for (let index = (messages || []).length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (!message) continue;
    if (
      !sawSlackContinuation &&
      typeof message.content === "string" &&
      message.content.trim() === SLACK_CONNECTED_CONTINUATION
    ) {
      sawSlackContinuation = true;
      continue;
    }
    if (message.capabilityId !== EXTENSION_ACTIVATE_CAPABILITY_ID) continue;
    if (message.toolStatus && message.toolStatus !== "success") continue;
    const sourceMessageId = message.id || null;
    const onboarding = onboardingFromExtensionActivateOutput(
      parseJsonObject(message.toolResultPreview),
      currentThreadId,
      sourceMessageId,
    );
    if (!onboarding) continue;
    // The user dismissed this specific activation's pairing panel; don't
    // re-derive it from the persistent tool-result card on the next render.
    if (sourceMessageId && dismissedSourceIds?.has?.(sourceMessageId)) {
      return null;
    }
    if (
      sawSlackContinuation &&
      String(onboarding.extensionName || "").toLowerCase() === "slack"
    ) {
      return null;
    }
    return onboarding;
  }
  return null;
}

function onboardingFromExtensionActivateOutput(output, threadId, sourceMessageId) {
  const payload = output?.payload;
  if (payload?.kind !== "extension_activate" || payload.activated !== true) {
    return null;
  }
  const message = typeof output.message === "string" ? output.message : "";
  if (!activationMessageRequiresPairing(message)) {
    return null;
  }
  const extensionName = output.package_ref?.id || output.packageRef?.id || null;
  if (!extensionName) return null;
  return {
    state: "pairing_required",
    extensionName,
    requestId: null,
    threadId,
    sourceMessageId: sourceMessageId || null,
    message,
    instructions: activationPairingInstructions(extensionName),
    setupUrl: null,
    inputPlaceholder:
      extensionName.toLowerCase() === "slack"
        ? "Enter Slack pairing code"
        : "Enter pairing code",
    submitLabel: "Connect",
  };
}

function parseJsonObject(value) {
  if (typeof value !== "string" || !value.trim()) return null;
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch (_) {
    return null;
  }
}

function activationMessageRequiresPairing(message) {
  const normalized = message.toLowerCase();
  return (
    normalized.includes("pairing") &&
    (normalized.includes("external channel") ||
      normalized.includes("inbound channel") ||
      normalized.includes("connection panel"))
  );
}

function activationPairingInstructions(extensionName) {
  if (extensionName.toLowerCase() === "slack") {
    return "Go to Slack and DM the IronClaw Reborn app to get a pairing code. Paste the code here; it will not be sent to the model.";
  }
  const displayName = extensionName
    .replace(/[-_]+/g, " ")
    .replace(/\b\w/g, (char) => char.toUpperCase());
  return `Open ${displayName}'s app or bot, get the pairing code or connection challenge, and paste it here.`;
}
