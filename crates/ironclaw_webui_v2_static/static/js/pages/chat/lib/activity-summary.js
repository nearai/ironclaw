export function summarizeActivity(activity) {
  let reasoning = 0;
  let tools = 0;
  let failed = 0;
  let running = 0;

  for (const item of activity) {
    if (item.role === "thinking") reasoning += 1;
    if (item.role === "tool_activity") {
      const summary = summarizeToolItems([item]);
      tools += summary.tools;
      failed += summary.failed;
      running += summary.running;
    }
    if (hasToolCalls(item)) {
      const summary = summarizeToolItems(item.toolCalls);
      tools += summary.tools;
      failed += summary.failed;
      running += summary.running;
    }
  }

  const parts = [];
  if (reasoning) parts.push(`${reasoning} reasoning`);
  if (tools) parts.push(`${tools} ${tools === 1 ? "tool" : "tools"}`);
  if (failed) parts.push(`${failed} failed`);
  if (!failed && running) parts.push("running");

  return {
    hasError: failed > 0,
    label: `Activity${parts.length ? ` - ${parts.join(", ")}` : ""}`,
  };
}

function summarizeToolItems(items) {
  let failed = 0;
  let running = 0;

  for (const item of items) {
    if (item.toolStatus === "error") failed += 1;
    if (item.toolStatus === "running") running += 1;
  }

  return { tools: items.length, failed, running };
}

function hasToolCalls(item) {
  return item.toolCalls && item.toolCalls.length > 0;
}
