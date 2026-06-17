import { Bot, Trash2 } from "lucide-react";
import type { ConversationThread } from "@/hooks/use-conversation";

interface SubagentRowProps {
  thread: ConversationThread;
  isActive: boolean;
  indented?: boolean;
  onClick: (threadId: string) => void;
  onDelete: (threadId: string) => void;
}

export function SubagentRow({ thread, isActive, indented = false, onClick, onDelete }: SubagentRowProps) {
  return (
    <div
      key={thread.threadId}
      role="button"
      tabIndex={0}
      onClick={() => onClick(thread.threadId)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onClick(thread.threadId);
        }
      }}
      className={`group flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-left text-xs transition-colors cursor-pointer touch-manipulation ${indented ? "pl-7" : ""} ${
        isActive
          ? "bg-accent text-accent-foreground"
          : "text-muted-foreground hover:bg-muted active:bg-muted"
      }`}
    >
      <Bot size={12} className="shrink-0" />
      <span className="flex-1 truncate">
        {thread.title ?? "Sub-agent"}
      </span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onDelete(thread.threadId);
        }}
        className="shrink-0 p-1 -m-1 opacity-0 group-hover:opacity-100 focus:opacity-100 transition-opacity touch-manipulation"
        aria-label="Delete sub-agent thread"
      >
        <Trash2
          size={10}
          className="text-muted-foreground/40 hover:text-destructive transition-colors"
        />
      </button>
    </div>
  );
}
