import type { ToolCallPart, ToolResultPart, UIMessage } from "@tanstack/ai";
import { AlertCircle, ChevronDown, Copy, FileIcon, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useApiClient } from "@/app";
import { ActivityRun } from "@/components/activity-run";
import { Markdown } from "@/components/ui/markdown";
import { cn } from "@/lib/utils";

function InlineAttachmentImage({
  threadId,
  messageId,
  attachmentId,
  mimeType,
  filename,
  inlineBase64,
}: {
  threadId: string;
  messageId: string;
  attachmentId: string;
  mimeType?: string;
  filename?: string;
  inlineBase64?: string;
}) {
  const apiClient = useApiClient();
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState(false);
  const blobUrlRef = useRef<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        if (inlineBase64) {
          const binaryStr = atob(inlineBase64);
          const bytes = new Uint8Array(binaryStr.length);
          for (let i = 0; i < binaryStr.length; i++) {
            bytes[i] = binaryStr.charCodeAt(i);
          }
          const blob = new Blob([bytes], { type: mimeType ?? "image/png" });
          const url = URL.createObjectURL(blob);
          blobUrlRef.current = url;
          if (!cancelled) setSrc(url);
          return;
        }
        const result = await apiClient.ironclaw.threads.getAttachment({
          id: threadId,
          messageId,
          attachmentId,
        });
        if (cancelled) return;
        const binaryStr = atob(result.contentBase64);
        const bytes = new Uint8Array(binaryStr.length);
        for (let i = 0; i < binaryStr.length; i++) {
          bytes[i] = binaryStr.charCodeAt(i);
        }
        const blob = new Blob([bytes], { type: result.mimeType });
        const url = URL.createObjectURL(blob);
        blobUrlRef.current = url;
        setSrc(url);
      } catch {
        if (!cancelled) setError(true);
      }
    })();
    return () => {
      cancelled = true;
      if (blobUrlRef.current) URL.revokeObjectURL(blobUrlRef.current);
    };
  }, [apiClient, threadId, messageId, attachmentId, inlineBase64]);

  if (error) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-destructive/20 bg-destructive/5 px-3 py-2 text-xs text-muted-foreground">
        <FileIcon size={14} />
        {filename ?? "attachment"}
      </div>
    );
  }

  if (!src) {
    return (
      <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-6">
        <Loader2 size={16} className="animate-spin text-muted-foreground" />
        <span className="text-xs text-muted-foreground">Loading image...</span>
      </div>
    );
  }

  return (
    <img
      src={src}
      alt={filename ?? "attachment"}
      className="max-h-64 w-auto rounded-lg object-cover"
    />
  );
}

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
  const isSystem = message.role === "system";
  const isFailed = status === "failed";
  const [copied, setCopied] = useState(false);
  const [detailsExpanded, setDetailsExpanded] = useState(false);

  const handleCopy = async (text: string) => {
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  const toolResultMap = new Map<string, ToolResultPart>();

  for (const p of message.parts) {
    if (p.type === "tool-result") toolResultMap.set(p.toolCallId, p);
  }

  const textContent = message.parts
    .filter((p): p is { type: "text"; content: string } => p.type === "text")
    .map((p) => p.content)
    .join(" ");

  const errorData = message.parts.find((p: any) => p.type === "error-data") as
    | { type: "error-data"; content: unknown }
    | undefined;

  if (isSystem) {
    return (
      <div className="flex items-start gap-2 rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 max-w-[85%] sm:max-w-[78%] lg:max-w-[70%]">
        <AlertCircle size={14} className="shrink-0 text-destructive mt-0.5" />
        <div className="min-w-0">
          <p className="text-xs font-semibold text-destructive">Error</p>
          <Markdown
            content={textContent}
            className="text-sm text-destructive/80 [&_p]:mb-0"
          />
          {errorData && (
            <div className="mt-2">
              <button
                type="button"
                onClick={() => setDetailsExpanded((v) => !v)}
                aria-expanded={detailsExpanded}
                className="flex items-center gap-1 text-xs text-destructive/60 hover:text-destructive transition-colors"
              >
                <ChevronDown
                  size={12}
                  className={detailsExpanded ? "rotate-0" : "-rotate-90"}
                />
                {detailsExpanded ? "Hide details" : "Error details"}
              </button>
              {detailsExpanded && (
                <pre className="mt-2 rounded bg-destructive/10 p-2 text-xs text-destructive/80 whitespace-pre-wrap overflow-x-auto max-h-48 overflow-y-auto">
                  {typeof errorData.content === "string"
                    ? errorData.content
                    : JSON.stringify(errorData.content, null, 2)}
                </pre>
              )}
            </div>
          )}
        </div>
      </div>
    );
  }

  const attachmentImages = (() => {
    const images: Array<{
      key: string;
      threadId: string;
      messageId: string;
      attachmentId: string;
      mimeType?: string;
      filename?: string;
      inlineBase64?: string;
    }> = [];
    for (const p of message.parts) {
      if (p.type !== "tool-call" || p.name !== "attachment") continue;
      const result = toolResultMap.get(p.id);
      if (!result) continue;
      try {
        const data: Record<string, unknown> = JSON.parse(
          typeof result.content === "string" ? result.content : "{}",
        );
        if (data.kind === "image" && typeof data.threadId === "string") {
          images.push({
            key: p.id,
            threadId: String(data.threadId),
            messageId: String(data.messageId ?? ""),
            attachmentId: String(data.attachmentId ?? ""),
            mimeType: typeof data.mimeType === "string" ? data.mimeType : undefined,
            filename: typeof data.filename === "string" ? data.filename : undefined,
            inlineBase64: typeof data.inlineBase64 === "string" ? data.inlineBase64 : undefined,
          });
        }
      } catch {}
    }
    return images;
  })();

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
        if (p.name === "attachment") {
          flushToolGroup();
          const result = toolResultMap.get(p.id);
          if (result) {
            try {
              const data = JSON.parse(typeof result.content === "string" ? result.content : "{}");
              if (data.kind === "image") {
                blocks.push(<InlineAttachmentImage key={p.id} {...data} />);
              }
            } catch {}
          }
          continue;
        }
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
    <div
      className={cn("group flex w-full gap-2", isUser ? "justify-end" : "justify-start items-end")}
    >
      {!isUser && (
        <img
          src="/logo.png"
          alt="IronClaw"
          className="shrink-0 w-6 h-6 sm:w-7 sm:h-7 mb-0.5 transition-transform duration-300 ease-out hover:scale-125 hover:-rotate-12 hover:drop-shadow-[0_0_8px_rgba(17,145,240,0.6)] cursor-pointer"
        />
      )}
      <div
        className={cn(
          "max-w-[85%] sm:max-w-[78%] lg:max-w-[70%] min-w-0",
          isUser
            ? "rounded-2xl rounded-br-sm bg-primary px-4 py-2.5 text-sm text-primary-foreground space-y-2"
            : "rounded-2xl rounded-bl-sm text-sm text-foreground bg-muted px-4 py-2.5 space-y-2",
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
          <>
            {textContent && (
              <p className="whitespace-pre-wrap break-words">{textContent}</p>
            )}
            {attachmentImages.map(({ key, ...img }) => (
              <InlineAttachmentImage key={key} {...img} />
            ))}
          </>
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
