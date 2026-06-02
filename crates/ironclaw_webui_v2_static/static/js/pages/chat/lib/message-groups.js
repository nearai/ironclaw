/* Collapse consecutive single tool_activity messages into runs for rendering.
   If the persisted timeline puts trailing reasoning/tool activity after the
   final assistant reply, render that activity before the reply so the answer
   stays last. Earlier assistant messages keep their original position because
   they are middle-of-turn narration. */
export function groupMessages(messages) {
  const ordered = assistantLastForTrailingActivity(messages);
  const items = [];
  let run = null;
  for (const msg of ordered) {
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
  return items;
}

function assistantLastForTrailingActivity(messages) {
  const lastAssistant = lastAssistantReplyIndex(messages);
  if (lastAssistant < 0 || lastAssistant === messages.length - 1) return messages;

  const trailing = messages.slice(lastAssistant + 1);
  if (!trailing.every(isAuxiliaryActivity)) return messages;

  return [
    ...messages.slice(0, lastAssistant),
    ...trailing,
    messages[lastAssistant],
  ];
}

function lastAssistantReplyIndex(messages) {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (isAssistantReply(messages[i])) return i;
  }
  return -1;
}

function isAssistantReply(msg) {
  return msg.role === "assistant" && !(msg.toolCalls && msg.toolCalls.length > 0);
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
