
export function isFinalAssistantMessage(message) {
  return message?.role === "assistant" && message?.isFinalReply === true;
}

export function isLiveAssistantMessage(message) {
  return (
    message?.role === "assistant" &&
    message?.isFinalReply === false &&
    typeof message?.turnRunId === "string" &&
    typeof message?.id === "string" &&
    message.id.startsWith("text-")
  );
}

export function isFinalAssistantForRun(message, runId) {
  return (
    isFinalAssistantMessage(message) &&
    (!runId || message?.turnRunId === runId)
  );
}

export function isRunActivityMessage(message) {
  return (
    message?.role === "tool_activity" ||
    message?.role === "thinking" ||
    message?.role === "narration"
  );
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

  let replacementIndex = currentMessages.findIndex(
    (message) =>
      isFinalAssistantMessage(message) && message.turnRunId === runId,
  );
  if (replacementIndex < 0) {
    // The streaming answer is the run's last live phase; narration (its
    // own role) is activity and never the answer.
    for (let index = currentMessages.length - 1; index >= 0; index -= 1) {
      const message = currentMessages[index];
      if (
        message?.role === "assistant" &&
        message.turnRunId === runId &&
        message.isFinalReply === false
      ) {
        replacementIndex = index;
        break;
      }
    }
  }

  if (replacementIndex < 0) {
    return [...currentMessages, replyMessage];
  }
  const next = [...currentMessages];
  next[replacementIndex] = finalReplyReplacement(
    currentMessages[replacementIndex],
    replyMessage,
  );
  // Earlier phases collapse into the final reply; narration is not an
  // assistant message and stays with the run's activity, the way reasoning
  // and tool cards do.
  return next.filter(
    (message, index) =>
      index === replacementIndex ||
      message?.role !== "assistant" ||
      message.turnRunId !== runId ||
      message.isFinalReply !== false,
  );
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
