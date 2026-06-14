import type { UIMessage } from "@tanstack/ai-react";
import { AlertCircle, CheckCircle2, Copy, Loader2, ShieldCheck, ShieldX, Terminal } from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Markdown } from "@/components/ui/markdown";
import { cn } from "@/lib/utils";

interface ChatMessageProps {
  message: UIMessage;
  isOptimistic?: boolean;
  status?: string;
  onApproveTool?: (toolCallId: string, approved: boolean) => void;
}

function ToolCallCard({
  name,
  state,
  output,
  approval,
  onApprove,
}: {
  name: string;
  state: string;
  output?: unknown;
  approval?: { id: string; needsApproval: boolean; approved?: boolean };
  onApprove?: (approved: boolean) => void;
}) {
  const isLoading = state === "input-streaming" || state === "input-complete";
  const isApproval = state === "approval-requested" && approval?.needsApproval;
  const isComplete = state === "complete";

  return (
    <div className="rounded-lg border border-border bg-muted/50 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        {isLoading ? <Loader2 size={12} className="animate-spin text-muted-foreground" /> : null}
        {isComplete ? <CheckCircle2 size={12} className="text-[color:var(--near-green)]" /> : null}
        {isApproval ? <ShieldCheck size={12} className="text-amber-500" /> : null}
        <span className="font-medium text-foreground">{name}</span>
      </div>
      {typeof output === "string" ? <p className="mt-1 text-muted-foreground">{output}</p> : null}
      {isApproval && onApprove ? (
        <div className="mt-2 flex gap-2">
          <Button
            size="sm"
            variant="default"
            className="h-7 text-xs"
            onClick={() => onApprove(true)}
          >
            <ShieldCheck size={12} className="mr-1" />
            Approve
          </Button>
          <Button
            size="sm"
            variant="outline"
            className="h-7 text-xs"
            onClick={() => onApprove(false)}
          >
            <ShieldX size={12} className="mr-1" />
            Deny
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function ToolResultCard({ content, state }: { content: string | unknown[]; state: string }) {
  const text = typeof content === "string" ? content : "";

  return (
    <div className="rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        <Terminal size={12} className="text-muted-foreground" />
        <span className="font-medium text-muted-foreground">
          {state === "error" ? "Error" : "Result"}
        </span>
      </div>
      {text && <p className="mt-1 text-muted-foreground/80 line-clamp-3">{text}</p>}
    </div>
  );
}

export function ChatMessage({ message, isOptimistic, status, onApproveTool }: ChatMessageProps) {
  const isUser = message.role === "user";
  const isFailed = status === "failed";
  const [copied, setCopied] = useState(false);

  const handleCopy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  let textContent = "";
  const toolCallParts: Array<{
    id: string;
    name: string;
    state: string;
    output?: unknown;
    approval?: { id: string; needsApproval: boolean; approved?: boolean };
  }> = [];
  const toolResultParts: Array<{
    toolCallId: string;
    content: string | unknown[];
    state: string;
  }> = [];
  const thinkingParts: Array<{ content: string }> = [];

  for (const p of message.parts) {
    const part = p as Record<string, unknown>;
    if (part.type === "text") {
      textContent = (part.content ?? part.text ?? "") as string;
    } else if (part.type === "tool-call") {
      toolCallParts.push({
        id: (part.id ?? part.toolCallId ?? "") as string,
        name: (part.name ?? part.toolCallName ?? part.toolName ?? "") as string,
        state: (part.state ?? "input-complete") as string,
        output: part.output,
        approval: part.approval as
          | { id: string; needsApproval: boolean; approved?: boolean }
          | undefined,
      });
    } else if (part.type === "tool-result") {
      toolResultParts.push({
        toolCallId: (part.toolCallId ?? "") as string,
        content: (part.content ?? "") as string | unknown[],
        state: (part.state ?? "complete") as string,
      });
    } else if (part.type === "thinking") {
      thinkingParts.push({ content: (part.content ?? "") as string });
    }
  }

  return (
    <div className={cn("group flex w-full", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[80%] min-w-0 space-y-2",
          isUser
            ? "rounded-2xl rounded-br-md bg-primary px-4 py-2.5 text-sm text-primary-foreground"
            : "rounded-2xl rounded-bl-md bg-muted px-4 py-2.5 text-sm text-foreground",
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
          <>
            {thinkingParts.map((part, i) => (
              <div
                key={i}
                className="rounded-lg border border-dashed border-muted-foreground/20 bg-muted/30 px-3 py-2 text-xs italic text-muted-foreground"
              >
                {part.content}
              </div>
            ))}
            {textContent && (
              <Markdown
                content={textContent}
                className="[&_p]:mb-0 [&_ul]:mb-0 [&_ol]:mb-0 [&_pre]:mb-0 [&_h1]:mt-0 [&_h1]:mb-0 [&_h2]:mt-0 [&_h2]:mb-0 [&_h3]:mt-0 [&_h3]:mb-0 [&_blockquote]:mb-0 [&_hr]:my-2"
              />
            )}
            {toolCallParts.map((part) => (
              <ToolCallCard
                key={part.id}
                name={part.name}
                state={part.state}
                output={part.output}
                approval={part.approval}
                onApprove={
                  onApproveTool ? (approved) => onApproveTool(part.id, approved) : undefined
                }
              />
            ))}
            {toolResultParts.map((part) => (
              <ToolResultCard key={part.toolCallId} content={part.content} state={part.state} />
            ))}
          </>
        )}
        {message.createdAt ? (
          <div
            className={cn("mt-1 flex items-center gap-1.5", isUser ? "justify-end" : "justify-start")}
          >
            {!isUser && textContent && (
              <button
                type="button"
                onClick={() => handleCopy(textContent)}
                className="opacity-0 group-hover:opacity-100 transition-opacity"
                title={copied ? "Copied!" : "Copy message"}
              >
                <Copy size={10} className={cn("text-muted-foreground/60 hover:text-muted-foreground transition-colors", copied && "text-muted-foreground")} />
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
