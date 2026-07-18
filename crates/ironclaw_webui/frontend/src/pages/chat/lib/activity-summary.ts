import { interpolateParams } from "../../../lib/i18n-format";

const FALLBACK_TRANSLATIONS = {
  "activity.title": "Activity",
  "activity.summaryWithParts": "Activity - {parts}",
  "activity.reasoning": "{count} reasoning",
  "activity.reasonings": "{count} reasoning",
  "activity.tool": "{count} tool",
  "activity.tools": "{count} tools",
  "activity.failed": "{count} failed",
  "activity.failedPlural": "{count} failed",
  "activity.declined": "{count} declined",
  "activity.declinedPlural": "{count} declined",
  "activity.running": "running",
  "activity.separator": ", ",
};

function fallbackT(key, params = {}) {
  const text = FALLBACK_TRANSLATIONS[key] || key;
  return interpolateParams(text, params);
}

export function summarizeActivity(activity, t = fallbackT) {
  let reasoning = 0;
  let tools = 0;
  let failed = 0;
  let declined = 0;
  let running = 0;

  for (const item of activity) {
    if (item.role === "thinking") reasoning += 1;
    if (item.role === "tool_activity") {
      const summary = summarizeToolItems([item]);
      tools += summary.tools;
      failed += summary.failed;
      declined += summary.declined;
      running += summary.running;
    }
    if (hasToolCalls(item)) {
      const summary = summarizeToolItems(item.toolCalls);
      tools += summary.tools;
      failed += summary.failed;
      declined += summary.declined;
      running += summary.running;
    }
  }

  const parts = [];
  if (reasoning) {
    parts.push(t(reasoning === 1 ? "activity.reasoning" : "activity.reasonings", {
      count: reasoning,
    }));
  }
  if (tools) parts.push(t(tools === 1 ? "activity.tool" : "activity.tools", { count: tools }));
  if (failed) {
    parts.push(t(failed === 1 ? "activity.failed" : "activity.failedPlural", {
      count: failed,
    }));
  }
  if (declined) {
    parts.push(t(declined === 1 ? "activity.declined" : "activity.declinedPlural", {
      count: declined,
    }));
  }
  if (!failed && !declined && running) parts.push(t("activity.running"));
  const localizedSeparator = t("activity.separator");
  const separator = localizedSeparator === "activity.separator" ? ", " : localizedSeparator;

  return {
    hasError: failed > 0,
    hasDeclined: declined > 0,
    label: parts.length
      ? t("activity.summaryWithParts", { parts: parts.join(separator) })
      : t("activity.title"),
  };
}

function summarizeToolItems(items) {
  let failed = 0;
  let declined = 0;
  let running = 0;

  for (const item of items) {
    if (item.toolStatus === "error") failed += 1;
    if (item.toolStatus === "declined") declined += 1;
    if (item.toolStatus === "running") running += 1;
  }

  return { tools: items.length, failed, declined, running };
}

function hasToolCalls(item) {
  return item.toolCalls && item.toolCalls.length > 0;
}
