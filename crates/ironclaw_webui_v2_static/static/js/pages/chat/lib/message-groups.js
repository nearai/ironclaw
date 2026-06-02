/* Collapse consecutive single tool_activity messages into runs for rendering.
   Messages that already carry toolCalls are full message bubbles and break a
   run; non-tool messages also break a run. */
export function groupMessages(messages) {
  const items = [];
  let run = null;
  for (const msg of messages) {
    const isToolActivity =
      msg.role === "tool_activity" && !(msg.toolCalls && msg.toolCalls.length > 0);
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
