import type { UIMessage } from "@tanstack/ai-react";
import { ChatMessage as UiChatMessage } from "@tanstack/ai-react-ui";
import {
  AlertCircle,
  CheckCircle2,
  ChevronDown,
  ChevronRight,
  Copy,
  File,
  Image,
  Loader2,
  Music,
  ShieldCheck,
  ShieldX,
  Terminal,
} from "lucide-react";
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Markdown } from "@/components/ui/markdown";
import { formatBytes } from "@/lib/attachments";
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
  const isComplete = state === "complete" || state === "output-available";
  const isError = state === "output-error";

  const outputText =
    typeof output === "string"
      ? output
      : output && typeof output === "object"
        ? JSON.stringify(output, null, 2)
        : "";

  return (
    <div className="rounded-lg border border-border bg-muted/50 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        {isLoading ? <Loader2 size={12} className="animate-spin text-muted-foreground" /> : null}
        {isComplete ? <CheckCircle2 size={12} className="text-[color:var(--near-green)]" /> : null}
        {isError ? <AlertCircle size={12} className="text-destructive" /> : null}
        {isApproval ? <ShieldCheck size={12} className="text-amber-500" /> : null}
        <span className="font-medium text-foreground">{name}</span>
      </div>
      {outputText ? (
        <pre className="mt-1 max-h-32 overflow-y-auto whitespace-pre-wrap font-mono text-muted-foreground">
          {outputText}
        </pre>
      ) : null}
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
  const [expanded, setExpanded] = useState(false);
  const raw = typeof content === "string" ? content : "";

  let displayText = raw;
  let outputKind: string | null = null;
  let isTruncated = false;

  if (raw) {
    try {
      const parsed = JSON.parse(raw);
      if (parsed && typeof parsed === "object") {
        displayText = parsed.output ?? parsed.text ?? parsed.result ?? raw;
        outputKind = parsed.output_kind ?? null;
        isTruncated = !!parsed.truncated;
      }
    } catch {
      // not JSON, use raw text as-is
    }
  }

  const lineCount = displayText.split("\n").length;
  const shouldCollapse = lineCount > 5 && !expanded;
  const display = shouldCollapse ? displayText.split("\n").slice(0, 5).join("\n") : displayText;

  return (
    <div className="rounded-lg border border-border bg-muted/30 px-3 py-2 text-xs">
      <div className="flex items-center gap-2">
        <Terminal size={12} className="text-muted-foreground" />
        <span className="font-medium text-muted-foreground">
          {state === "error" ? "Error" : "Result"}
        </span>
        {outputKind ? (
          <span className="rounded bg-muted-foreground/10 px-1.5 py-0.5 text-[10px] uppercase text-muted-foreground/70 font-mono">
            {outputKind}
          </span>
        ) : null}
        {isTruncated ? (
          <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-[10px] text-amber-600">
            truncated
          </span>
        ) : null}
      </div>
      {display ? (
        <pre className="mt-1 max-h-96 overflow-y-auto whitespace-pre-wrap font-mono text-muted-foreground/80">
          {display}
        </pre>
      ) : null}
      {shouldCollapse ? (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="mt-1 flex items-center gap-1 text-[10px] text-muted-foreground/60 hover:text-muted-foreground transition-colors"
        >
          <ChevronRight size={10} />
          Show more ({lineCount - 5} more lines)
        </button>
      ) : null}
      {expanded && lineCount > 5 ? (
        <button
          type="button"
          onClick={() => setExpanded(false)}
          className="mt-1 flex items-center gap-1 text-[10px] text-muted-foreground/60 hover:text-muted-foreground transition-colors"
        >
          <ChevronDown size={10} />
          Show less
        </button>
      ) : null}
    </div>
  );
}

function AttachmentCard({ attachment }: { attachment: any }) {
  const kind = attachment.kind as string;
  const isImage = kind === "image" || attachment.mimeType?.startsWith("image/");
  const isAudio = kind === "audio" || attachment.mimeType?.startsWith("audio/");
  const previewUrl =
    (attachment.previewUrl ?? attachment.dataBase64)
      ? `data:${attachment.mimeType};base64,${attachment.dataBase64}`
      : null;

  const icon = isImage ? Image : isAudio ? Music : File;
  const Icon = icon;

  const body = (
    <div className="flex items-center gap-2 overflow-hidden rounded-md border border-border bg-muted/30 px-2.5 py-1.5 text-xs">
      <Icon size={14} className="shrink-0 text-muted-foreground" />
      <span
        className="min-w-0 flex-1 truncate text-foreground"
        title={attachment.filename ?? attachment.id}
      >
        {attachment.filename ?? "Unknown file"}
      </span>
      {attachment.mimeType ? (
        <span className="shrink-0 text-muted-foreground/70">{attachment.mimeType}</span>
      ) : null}
      {attachment.sizeBytes != null ? (
        <span className="shrink-0 text-muted-foreground/70">
          {formatBytes(attachment.sizeBytes)}
        </span>
      ) : null}
      {attachment.extractedText ? (
        <span className="max-w-40 truncate text-muted-foreground/50 italic">
          {attachment.extractedText}
        </span>
      ) : null}
    </div>
  );

  if (isImage && previewUrl) {
    return (
      <a href={previewUrl} target="_blank" rel="noreferrer" className="group block">
        <div className="overflow-hidden rounded-md border border-border bg-muted/20">
          <img
            src={previewUrl}
            alt={attachment.filename ?? "attachment"}
            className="max-h-32 w-auto object-contain transition-opacity group-hover:opacity-80"
          />
        </div>
        {body}
      </a>
    );
  }

  return body;
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

  const textContent = message.parts?.length
    ? message.parts
        .filter((p) => p.type === "text")
        .map((p) => (p as any).content ?? (p as any).text ?? "")
        .join(" ")
    : ((message as any).content ?? "");

  const attachments = (message as any).attachments as any[] | undefined;

  return (
    <div className={cn("group flex w-full", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[90%] sm:max-w-[80%] lg:max-w-[70%] min-w-0 space-y-2",
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
          <UiChatMessage
            message={message}
            className="space-y-2"
            defaultToolRenderer={({ id, name, state, output, approval }) => (
              <ToolCallCard
                name={name}
                state={state}
                output={output}
                approval={approval}
                onApprove={onApproveTool ? (approved) => onApproveTool(id, approved) : undefined}
              />
            )}
            toolResultRenderer={({ toolCallId: _toolCallId, content, state }) => (
              <ToolResultCard content={content} state={state} />
            )}
            thinkingPartRenderer={({ content }) => (
              <div className="rounded-lg border border-dashed border-muted-foreground/20 bg-muted/30 px-3 py-2 text-xs italic text-muted-foreground">
                {content}
              </div>
            )}
            textPartRenderer={({ content }) => (
              <Markdown
                content={content}
                className="[&_p]:mb-0 [&_ul]:mb-0 [&_ol]:mb-0 [&_pre]:mb-0 [&_h1]:mt-0 [&_h1]:mb-0 [&_h2]:mt-0 [&_h2]:mb-0 [&_h3]:mt-0 [&_h3]:mb-0 [&_blockquote]:mb-0 [&_hr]:my-2"
              />
            )}
          />
        )}
        {attachments && attachments.length > 0 ? (
          <div className="space-y-1">
            {attachments.map((att: any) => (
              <AttachmentCard key={att.id ?? att.filename} attachment={att} />
            ))}
          </div>
        ) : null}
        {!isUser && message.createdAt ? (
          <div className="flex items-center gap-1.5 justify-start">
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
