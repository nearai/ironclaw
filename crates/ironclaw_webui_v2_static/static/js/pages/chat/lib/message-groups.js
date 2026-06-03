/* Collapse ordered reasoning/tool events into one activity run. If delayed
   activity arrives immediately after a finalized assistant answer, render that
   activity before the answer so the answer still closes its turn, including
   when a later user follow-up has already been appended. */
export function groupMessages(messages) {
  const items = [];

  for (let index = 0; index < messages.length; index += 1) {
    const msg = messages[index];

    if (isFinalAssistantReply(msg)) {
      const activity = followingActivity(messages, index + 1);
      const boundary = messages[index + 1 + activity.length];
      if (activity.length > 0 && (!boundary || boundary.role === "user")) {
        appendActivityRun(items, activity);
        appendMessage(items, msg);
        index += activity.length;
        continue;
      }
    }

    if (isActivity(msg)) {
      const activity = followingActivity(messages, index);
      appendActivityRun(items, activity);
      index += activity.length - 1;
      continue;
    }

    appendMessage(items, msg);
  }

  return items;
}

function followingActivity(messages, start) {
  let end = start;
  while (end < messages.length && isActivity(messages[end])) {
    end += 1;
  }
  return messages.slice(start, end);
}

function appendActivityRun(items, activity) {
  if (activity.length === 0) return;
  items.push({
    type: "activity-run",
    id: `activity-run-${activity[0].id}`,
    activity,
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
  return msg.toolCalls && msg.toolCalls.length > 0;
}
