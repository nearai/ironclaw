import { Bot, Trash2 } from "lucide-react";
import type { ConversationThread } from "@/hooks/use-conversation";

interface SubagentRowProps {
  thread: ConversationThread;
  onDelete: (threadId: string) => void;
}

export function SubagentRow({ thread, onDelete }: SubagentRowProps) {
  return (
    <>
      <Bot size={12} className="shrink-0" />
      <span className="flex-1 truncate text-xs">
        {thread.title ?? "Sub-agent"}
      </span>
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
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
    </>
  );
}
