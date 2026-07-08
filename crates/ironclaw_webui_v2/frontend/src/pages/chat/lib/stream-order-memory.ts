// @ts-nocheck

export function isFinalAssistantMessage(message) {
  return message?.role === "assistant" && message?.isFinalReply === true;
}

export function isFinalAssistantForRun(message, runId) {
  return (
    isFinalAssistantMessage(message) &&
    (!runId || message?.turnRunId === runId)
  );
}

export function isRunActivityMessage(message) {
  return message?.role === "tool_activity" || message?.role === "thinking";
}

export function carryFinalAssistantOrderFlags(fresh, current) {
  const keepAfterActivityRuns = new Set();
  for (const message of current || []) {
    if (
      isFinalAssistantMessage(message) &&
      message.keepFollowingActivityAfter === true &&
      typeof message.turnRunId === "string"
    ) {
      keepAfterActivityRuns.add(message.turnRunId);
    }
  }
  if (keepAfterActivityRuns.size === 0) return fresh;

  return (fresh || []).map((message) => {
    const runId = typeof message?.turnRunId === "string" ? message.turnRunId : null;
    if (!runId || !keepAfterActivityRuns.has(runId) || !isFinalAssistantMessage(message)) {
      return message;
    }
    return { ...message, keepFollowingActivityAfter: true };
  });
}

export function replaceAssistantReplyForRun(messages, replyMessage, runId) {
  const currentMessages = messages || [];
  if (!runId) return [...currentMessages, replyMessage];

  let replaced = false;
  let changed = false;
  const next = [];
  for (const message of currentMessages) {
    if (message?.role === "assistant" && message.turnRunId === runId) {
      if (!replaced) {
        replaced = true;
        changed = true;
        next.push(finalReplyReplacement(message, replyMessage));
      } else {
        changed = true;
      }
      continue;
    }
    next.push(message);
  }

  return replaced && changed ? next : [...currentMessages, replyMessage];
}

function finalReplyReplacement(current, replyMessage) {
  const replacement = {
    ...replyMessage,
    id: current.isFinalReply === true ? current.id : replyMessage.id,
    timestamp:
      current.isFinalReply === true && current.timestamp
        ? current.timestamp
        : replyMessage.timestamp,
  };
  if (
    current.keepFollowingActivityAfter === true ||
    replyMessage.keepFollowingActivityAfter === true
  ) {
    replacement.keepFollowingActivityAfter = true;
  }
  return replacement;
}
