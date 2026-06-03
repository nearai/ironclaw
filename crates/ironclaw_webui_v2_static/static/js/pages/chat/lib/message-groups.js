/* Collapse consecutive single tool_activity messages into runs for rendering.
   If the persisted timeline puts trailing reasoning/tool activity after the
   final assistant reply, render that activity before the reply so the answer
   stays last. Earlier assistant messages keep their original position because
   they are middle-of-turn narration. */
export function groupMessages(messages) {
  const items = [];
  const reordered = trailingActivityBeforeFinalReply(messages);
  if (reordered) {
    appendGroupedMessages(items, reordered.before);
    appendGroupedMessages(items, reordered.activity);
    appendGroupedMessages(items, [reordered.reply]);
    return items;
  }

  appendGroupedMessages(items, messages);
  return items;
}

function appendGroupedMessages(items, messages) {
  let run = null;
  for (const msg of messages) {
    const isToolActivity = isCollapsibleToolActivity(msg);
    if (isToolActivity) {
      if (!run) {
        run = { type: "tool-run", id: `tool-run-${msg.id}`, tools: [] };
        items.push(run);
      }
      run.tools.push(msg);
    } else {
      run = null;
      items.push({ type: "message", id: msg.id, message: msg });
    }
  }
}

function trailingActivityBeforeFinalReply(messages) {
  const lastAssistant = lastFinalAssistantReplyIndex(messages);
  if (lastAssistant < 0 || lastAssistant === messages.length - 1) return null;

  const trailing = messages.slice(lastAssistant + 1);
  if (!trailing.every(isAuxiliaryActivity)) return null;

  return {
    before: messages.slice(0, lastAssistant),
    activity: trailing,
    reply: messages[lastAssistant],
  };
}

function lastFinalAssistantReplyIndex(messages) {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (isFinalAssistantReply(messages[i])) return i;
  }
  return -1;
}

function isFinalAssistantReply(msg) {
  return (
    msg.role === "assistant" &&
    !(msg.toolCalls && msg.toolCalls.length > 0) &&
    (msg.isFinalReply === true ||
      ((msg.kind === "assistant" || msg.kind === "assistant_message") &&
        msg.status === "finalized"))
  );
}

function isAuxiliaryActivity(msg) {
  return msg.role === "thinking" || msg.role === "tool_activity" || hasToolCalls(msg);
}

function isCollapsibleToolActivity(msg) {
  return msg.role === "tool_activity" && !hasToolCalls(msg);
}

function hasToolCalls(msg) {
  return msg.toolCalls && msg.toolCalls.length > 0;
}
