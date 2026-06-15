import type { ToolCallPart, ToolResultPart, UIMessage } from "@tanstack/ai";
import { AlertCircle, Copy } from "lucide-react";
import { useState } from "react";
import { ActivityRun } from "@/components/activity-run";
import { Markdown } from "@/components/ui/markdown";
import { cn } from "@/lib/utils";

interface ChatMessageProps {
  message: UIMessage;
  isOptimistic?: boolean;
  status?: string;
  verbose?: boolean;
}

function RenderedBlocks({ parts, messageId }: { parts: React.ReactNode[]; messageId: string }) {
  return (
    <div className="space-y-2">
      {parts.map((block, i) => (
        <div key={`${messageId}-block-${i}`}>{block}</div>
      ))}
    </div>
  );
}

export function ChatMessage({ message, isOptimistic, status, verbose }: ChatMessageProps) {
  const isUser = message.role === "user";
  const isFailed = status === "failed";
  const [copied, setCopied] = useState(false);

  const handleCopy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const toolResultMap = new Map<string, ToolResultPart>();
  const allToolCalls: ToolCallPart[] = [];

  for (const p of message.parts) {
    if (p.type === "tool-result") toolResultMap.set(p.toolCallId, p);
    if (p.type === "tool-call") allToolCalls.push(p);
  }

  const textContent = message.parts
    .filter((p): p is { type: "text"; content: string } => p.type === "text")
    .map((p) => p.content)
    .join(" ");

  function buildBlocks(): React.ReactNode[] {
    const blocks: React.ReactNode[] = [];
    let toolGroup: ToolCallPart[] = [];

    function flushToolGroup() {
      if (toolGroup.length === 0) return;
      const tools = toolGroup.map((call) => ({
        call,
        result: toolResultMap.get(call.id),
      }));
      blocks.push(<ActivityRun key={`tools-${toolGroup[0].id}`} tools={tools} verbose={verbose} />);
      toolGroup = [];
    }

    for (const p of message.parts) {
      if (p.type === "tool-call") {
        toolGroup.push(p);
        continue;
      }
      if (p.type === "tool-result") continue; // handled via toolResultMap

      flushToolGroup();

      if (p.type === "text") {
        blocks.push(
          <Markdown
            key={`${message.id}-text-${blocks.length}`}
            content={p.content}
            className="[&_p]:mb-0 [&_ul]:mb-0 [&_ol]:mb-0 [&_pre]:mb-0 [&_h1]:mt-0 [&_h1]:mb-0 [&_h2]:mt-0 [&_h2]:mb-0 [&_h3]:mt-0 [&_h3]:mb-0 [&_blockquote]:mb-0 [&_hr]:my-2"
          />,
        );
        continue;
      }

      if (p.type === "thinking") {
        blocks.push(
          <div
            key={`${message.id}-thinking-${blocks.length}`}
            className="rounded-lg border border-dashed border-muted-foreground/20 bg-muted/30 px-3 py-2 text-xs italic text-muted-foreground"
          >
            {p.content}
          </div>,
        );
        continue;
      }
    }

    flushToolGroup();
    return blocks;
  }

  const blocks = buildBlocks();

  return (
    <div className={cn("group flex w-full", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[90%] sm:max-w-[80%] lg:max-w-[70%] min-w-0",
          isUser
            ? "rounded-2xl rounded-br-md bg-primary px-4 py-2.5 text-sm text-primary-foreground space-y-2"
            : "rounded-2xl rounded-bl-md text-sm text-foreground bg-muted px-4 py-2.5 space-y-2",
          isOptimistic && "opacity-70",
          isFailed && "border border-destructive/50 bg-destructive/5",
        )}
      >
        {isFailed && (
          <div className="flex items-center gap-1.5 text-xs text-destructive">
            <AlertCircle size={12} />
            <span>Failed to send</span>
          </div>
        )}
        {isUser ? (
          <p className="whitespace-pre-wrap break-words">{textContent}</p>
        ) : (
          <RenderedBlocks parts={blocks} messageId={message.id} />
        )}
        {!isUser && message.createdAt ? (
          <div className="flex items-center gap-1.5 justify-start pt-0.5">
            {textContent && (
              <button
                type="button"
                onClick={() => handleCopy(textContent)}
                className="opacity-0 group-hover:opacity-100 transition-opacity"
                title={copied ? "Copied!" : "Copy message"}
              >
                <Copy
                  size={10}
                  className={cn(
                    "text-muted-foreground/60 hover:text-muted-foreground transition-colors",
                    copied && "text-muted-foreground",
                  )}
                />
              </button>
            )}
            <span className="text-[10px] text-muted-foreground/60">
              {new Date(message.createdAt).toLocaleTimeString(undefined, {
                hour: "2-digit",
                minute: "2-digit",
              })}
            </span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
