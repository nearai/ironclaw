import { Icon } from "../../../design-system/icons";
import React from "react";
import { useT } from "../../../lib/i18n";
import { summarizeActivity } from "../lib/activity-summary";
import {
  messageBelongsToActiveRun,
  type ChatMessage,
} from "../lib/message-types";
import { MarkdownRenderer } from "./markdown-renderer";
import { ToolActivity } from "./tool-activity";

type ActivityRunProps = {
  activity: ChatMessage[];
  activeRunId?: string | null;
};

type ActivityItemProps = {
  item: ChatMessage;
  activeRunId: string | null;
};

type NoteItemProps = {
  icon: "spark" | "chat";
  content?: string;
  streaming?: boolean;
};

export function ActivityRun({ activity, activeRunId = null }: ActivityRunProps) {
  const t = useT();
  const summary = React.useMemo(() => summarizeActivity(activity, t), [activity, t]);
  const [expanded, setExpanded] = React.useState(false);

  return (
    <div className="mr-auto flex w-full min-w-0 flex-col v2-chat-readable-width" data-testid="activity-run">
      <button
        type="button"
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded ? "true" : "false"}
        data-testid="activity-run-toggle"
        className="v2-button flex w-full min-w-0 items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm text-iron-400 hover:text-iron-200"
      >
        <Icon name="layers" className="h-4 w-4 shrink-0" />
        <span className="min-w-0 truncate">{summary.label}</span>
        {summary.hasError &&
        (<Icon
          name="alert"
          className="h-3.5 w-3.5 shrink-0 text-[var(--v2-warning-text)]"
        />)}
        <Icon
          name="chevron"
          className={["ml-auto h-3.5 w-3.5 shrink-0", expanded ? "rotate-180" : ""].join(" ")}
        />
      </button>

      {expanded &&
      (
        <div className="mt-2 flex min-w-0 flex-col gap-3" data-testid="activity-run-items">
          {activity.map((item, index) => (
            <ActivityItem
              key={item.id || `${item.role || "activity"}-${index}`}
              item={item}
              activeRunId={activeRunId}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function ActivityItem({ item, activeRunId }: ActivityItemProps) {
  if (item.role === "thinking") {
    const isStreaming =
      messageBelongsToActiveRun(item, activeRunId);
    return (
      <NoteItem
        icon="spark"
        content={item.content}
        streaming={isStreaming}
      />
    );
  }

  if (item.role === "assistant" && item.isNarration === true) {
    // A model call the loop went on past: what the assistant said before
    // the tool call that followed it. Settled text, never streaming.
    return <NoteItem icon="chat" content={item.content} />;
  }

  if (item.role === "tool_activity" || hasToolCalls(item)) {
    const activity = hasToolCalls(item)
      ? { id: item.id, toolCalls: item.toolCalls }
      : item;
    return (<ToolActivity activity={activity} />);
  }

  return null;
}

function NoteItem({ icon, content, streaming = false }: NoteItemProps) {
  if (!content) return null;
  return (
    <div className="flex min-w-0 gap-3" data-testid={`activity-${icon === "spark" ? "reasoning" : "narration"}`}>
      <div
        className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-white/10 bg-iron-800 text-iron-100"
      >
        <Icon name={icon} className="h-4 w-4" />
      </div>
      <div className="min-w-0 flex-1 border-l-2 border-white/10 pl-3 text-iron-300 v2-chat-readable-width">
        <MarkdownRenderer
          content={content}
          className="text-[13px]"
          streaming={streaming}
        />
      </div>
    </div>
  );
}

function hasToolCalls(item) {
  return item?.toolCalls && item.toolCalls.length > 0;
}
