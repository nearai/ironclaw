import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { summarizeActivity } from "../lib/activity-summary.js";
import { MarkdownRenderer } from "./markdown-renderer.js";
import { ToolActivity } from "./tool-activity.js";

export function ActivityRun({ activity }) {
  const summary = summarizeActivity(activity);
  const shouldAutoExpand = shouldExpandActivityRun(activity);
  const [expanded, setExpanded] = React.useState(shouldAutoExpand);

  React.useEffect(() => {
    if (shouldAutoExpand) setExpanded(true);
  }, [shouldAutoExpand]);

  return html`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${() => setExpanded((value) => !value)}
        aria-expanded=${expanded ? "true" : "false"}
        className=${[
          "v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",
          summary.hasError
            ? "text-[var(--v2-danger-text)]"
            : "text-iron-400 hover:text-iron-200",
        ].join(" ")}
      >
        <${Icon} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${summary.label}</span>
        <${Icon}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0", expanded ? "rotate-180" : ""].join(" ")}
        />
      </button>

      ${expanded &&
      html`
        <div className="mt-2 flex flex-col gap-3">
          ${activity.map((item, index) => html`
            <${ActivityItem}
              key=${item.id || `${item.role || "activity"}-${index}`}
              item=${item}
            />
          `)}
        </div>
      `}
    </div>
  `;
}

function ActivityItem({ item }) {
  if (item.role === "thinking") {
    return html`<${ReasoningItem} content=${item.content} />`;
  }

  if (item.role === "tool_activity" || hasToolCalls(item)) {
    const activity = hasToolCalls(item)
      ? { id: item.id, toolCalls: item.toolCalls }
      : item;
    return html`<${ToolActivity} activity=${activity} />`;
  }

  return null;
}

function ReasoningItem({ content }) {
  if (!content) return null;
  return html`
    <div className="flex gap-3">
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <${Icon} name="spark" className="h-4 w-4" />
      </div>
      <div className="min-w-0 max-w-[85%] flex-1 border-l-2 border-white/10 pl-3 text-iron-300">
        <${MarkdownRenderer} content=${content} className="text-[13px]" />
      </div>
    </div>
  `;
}

function hasToolCalls(item) {
  return item?.toolCalls && item.toolCalls.length > 0;
}

function shouldExpandActivityRun(activity) {
  return (activity || []).some((item) => {
    if (item?.role === "thinking") return true;
    if (
      item?.toolStatus === "running" ||
      item?.toolStatus === "error" ||
      item?.toolStatus === "declined"
    ) {
      return true;
    }
    if (!hasToolCalls(item)) return false;
    return item.toolCalls.some(
      (tool) =>
        tool?.toolStatus === "running" ||
        tool?.toolStatus === "error" ||
        tool?.toolStatus === "declined",
    );
  });
}
