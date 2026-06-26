/* Collapse ordered reasoning/tool events into one activity run. If delayed
   activity arrives immediately after a finalized assistant answer, render that
   activity before the answer so the answer still closes its turn, including
   when a later user follow-up has already been appended. */
export function groupMessages(messages) {
  const orderedMessages = moveDelayedActivityBeforeFinalReply(messages);
  const items = [];

  for (let index = 0; index < orderedMessages.length; index += 1) {
    const msg = orderedMessages[index];

    if (isFinalAssistantReply(msg)) {
      const activity = followingActivity(orderedMessages, index + 1);
      const boundary = orderedMessages[index + 1 + activity.length];
      if (activity.length > 0 && (!boundary || boundary.role === "user")) {
        appendActivityRun(items, activity);
        appendMessage(items, msg);
        index += activity.length;
        continue;
      }
    }

    if (isActivity(msg)) {
      const activity = followingActivity(orderedMessages, index);
      appendActivityRun(items, activity);
      index += activity.length - 1;
      continue;
    }

    appendMessage(items, msg);
  }

  return items;
}

function moveDelayedActivityBeforeFinalReply(messages) {
  const finalReplyByRun = new Map();
  for (let index = 0; index < messages.length; index += 1) {
    const msg = messages[index];
    const runId = turnRunIdForMessage(msg);
    if (runId && isFinalAssistantReply(msg)) {
      finalReplyByRun.set(runId, index);
    }
  }
  if (finalReplyByRun.size === 0) return messages;

  const delayedByFinalIndex = new Map();
  const delayedIndexes = new Set();
  for (let index = 0; index < messages.length; index += 1) {
    const msg = messages[index];
    if (!isActivity(msg)) continue;
    const runId = turnRunIdForMessage(msg);
    const finalIndex = runId ? finalReplyByRun.get(runId) : undefined;
    if (finalIndex === undefined || finalIndex >= index) continue;

    const delayed = delayedByFinalIndex.get(finalIndex) || [];
    delayed.push(msg);
    delayedByFinalIndex.set(finalIndex, delayed);
    delayedIndexes.add(index);
  }
  if (delayedIndexes.size === 0) return messages;

  const ordered = [];
  for (let index = 0; index < messages.length; index += 1) {
    if (delayedIndexes.has(index)) continue;
    const delayed = delayedByFinalIndex.get(index);
    if (delayed) ordered.push(...delayed);
    ordered.push(messages[index]);
  }
  return ordered;
}

function followingActivity(messages, start) {
  let end = start;
  const runId = turnRunIdForMessage(messages[start]);
  while (
    end < messages.length &&
    isActivity(messages[end]) &&
    sameActivityRun(runId, messages[end])
  ) {
    end += 1;
  }
  return messages.slice(start, end);
}

function sameActivityRun(referenceRunId, msg) {
  const runId = turnRunIdForMessage(msg);
  return !referenceRunId || !runId || runId === referenceRunId;
}

function appendActivityRun(items, activity) {
  if (activity.length === 0) return;
  const orderedActivity = orderActivityRun(activity);
  items.push({
    type: "activity-run",
    id: `activity-run-${orderedActivity[0].id}`,
    activity: orderedActivity,
  });
}

function appendMessage(items, message) {
  items.push({ type: "message", id: message.id, message });
}

function isFinalAssistantReply(msg) {
  return (
    msg.role === "assistant" &&
    !hasToolCalls(msg) &&
    (msg.isFinalReply === true ||
      ((msg.kind === "assistant" || msg.kind === "assistant_message") &&
        msg.status === "finalized"))
  );
}

function isActivity(msg) {
  return msg.role === "thinking" || msg.role === "tool_activity" || hasToolCalls(msg);
}

function hasToolCalls(msg) {
  return msg?.toolCalls && msg.toolCalls.length > 0;
}

function turnRunIdForMessage(msg) {
  return msg?.turnRunId || null;
}

function orderActivityRun(activity) {
  return [...activity].sort((left, right) => {
    if (left?.role !== "tool_activity" || right?.role !== "tool_activity") {
      return 0;
    }
    return compareToolActivityOrder(left, right);
  });
}

function compareToolActivityOrder(left, right) {
  if (Number.isFinite(left.activityOrder) && Number.isFinite(right.activityOrder)) {
    const explicitOrder = left.activityOrder - right.activityOrder;
    if (explicitOrder !== 0) return explicitOrder;
  }

  const timestampOrder = compareNullableNumber(
    timestampMs(left.updatedAt || left.timestamp),
    timestampMs(right.updatedAt || right.timestamp),
  );
  if (timestampOrder !== 0) return timestampOrder;

  return compareNullableNumber(left.sequence, right.sequence);
}

function compareNullableNumber(left, right) {
  const leftNumber = Number.isFinite(left) ? left : null;
  const rightNumber = Number.isFinite(right) ? right : null;
  if (leftNumber === null && rightNumber === null) return 0;
  if (leftNumber === null) return 1;
  if (rightNumber === null) return -1;
  return leftNumber - rightNumber;
}

function timestampMs(value) {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}
