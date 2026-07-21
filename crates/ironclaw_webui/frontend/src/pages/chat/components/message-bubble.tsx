import React from "react";
import { MarkdownRenderer } from "./markdown-renderer";
import { ToolActivity } from "./tool-activity";
import { Icon } from "@ironclaw/design-system";
import { toast } from "../../../lib/toast";
import { ProjectFileChips } from "./project-file-chips";
import { AttachmentChip } from "./attachment-chip";
import { AttachmentPreviewModal } from "./attachment-preview";
import { useT } from "../../../lib/i18n";
import { fetchRunArtifact } from "../../../lib/api";
import { saveBlob } from "../../../lib/download";
import {
  CHAT_MESSAGE_ROLES,
  type ChatAttachment,
  type ChatMessage,
} from "../lib/message-types";

/* User keeps a tinted bubble; assistant is borderless (document-like);
   system stays as a centered notice, and error renders as an inline
   assistant-side alert. Reasoning ("thinking") renders as a collapsible
   disclosure (see ThinkingDisclosure). */
const ROLE_STYLES = {
  [CHAT_MESSAGE_ROLES.USER]:
    "ml-auto rounded-[18px] border border-[var(--v2-accent)]/25 bg-[var(--v2-accent-soft)] px-4 py-3 text-[var(--v2-text-strong)]",
  [CHAT_MESSAGE_ROLES.ASSISTANT]: "mr-auto px-1 text-[var(--v2-text-strong)]",
  [CHAT_MESSAGE_ROLES.SYSTEM]:
    "mx-auto rounded-[18px] border border-[var(--v2-warning-text)]/20 bg-[var(--v2-warning-soft)] px-4 py-3 text-center text-[var(--v2-warning-text)]",
  // The trailing `text-red-200` is pinned by message-bubble.test.ts
  // ("error role should align with the assistant-side chat stream").
  [CHAT_MESSAGE_ROLES.ERROR]:
    "mr-auto rounded-[18px] border border-[var(--v2-danger-text)]/25 bg-[var(--v2-danger-soft)] px-4 py-3 text-left text-red-200",
};

type MessageBubbleProps = {
  message: ChatMessage;
  onRetry?: (message: ChatMessage) => void;
  threadId?: string | null;
};

function formatTimestamp(value?: string) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/* Collapsible provider-reasoning summary. Collapsed by default so the
   thread stays clean; expands to the full reasoning markdown. Data comes
   from the `thinking` projection item (PR #4230). */
function ThinkingDisclosure({ content }: { content?: string }) {
  const t = useT();
  const [open, setOpen] = React.useState(false);
  if (!content) return null;
  return (
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open ? "true" : "false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-[var(--v2-text-faint)] hover:text-[var(--v2-text)]"
      >
        <Icon name="spark" className="h-3.5 w-3.5" />
        <span>{open ? t("chat.hideReasoning") : t("chat.reasoning")}</span>
        <Icon
          name="chevron"
          className={["h-3 w-3", open ? "rotate-180" : ""].join(" ")}
        />
      </button>
      {open &&
      (
        <div className="mt-1 border-l-2 border-[var(--v2-panel-border)] pl-3 text-[var(--v2-text-muted)]">
          <MarkdownRenderer content={content} className="text-[13px]" />
        </div>
      )}
    </div>
  );
}

function MessageBubbleImpl({ message, onRetry, threadId }: MessageBubbleProps) {
  const t = useT();
  const { role, content, images, attachments, generatedImages, isOptimistic, status, error, toolCalls, timestamp } = message;
  const isUser = role === CHAT_MESSAGE_ROLES.USER;
  const finalReplyState =
    role === CHAT_MESSAGE_ROLES.ASSISTANT &&
    typeof message.isFinalReply === "boolean"
      ? String(message.isFinalReply)
      : undefined;
  const failureCategory =
    role === CHAT_MESSAGE_ROLES.ERROR &&
    typeof message.failureCategory === "string"
      ? message.failureCategory
      : undefined;
  const failureStatus =
    role === CHAT_MESSAGE_ROLES.ERROR &&
    typeof message.failureStatus === "string"
      ? message.failureStatus
      : undefined;
  const [copied, setCopied] = React.useState(false);
  const [artifactDownloading, setArtifactDownloading] = React.useState(false);
  // The attachment currently open in the preview modal (null when closed).
  const [previewAttachment, setPreviewAttachment] =
    React.useState<ChatAttachment | null>(null);
  // All hooks must run before the role-based early returns below.
  // A message can change role in place across renders (e.g. an
  // optimistic bubble upgrading, or a streaming role shift), so
  // declaring `copy` after the early returns made the hook count
  // jump between renders and crashed the thread with "Rendered more
  // hooks than during the previous render". Keep every hook here.
  const copy = React.useCallback(async () => {
    try {
      await navigator.clipboard.writeText(typeof content === "string" ? content : "");
      setCopied(true);
      toast(t("common.copiedToClipboard"), { tone: "success" });
      setTimeout(() => setCopied(false), 1400);
    } catch {
      // clipboard unavailable — no-op
    }
  }, [content, t]);
  const turnRunId =
    typeof message.turnRunId === "string" ? message.turnRunId : "";
  const downloadArtifact = React.useCallback(async () => {
    if (!threadId || !turnRunId || artifactDownloading) return;
    setArtifactDownloading(true);
    try {
      const artifact = await fetchRunArtifact({ threadId, runId: turnRunId });
      const filenameRunId = turnRunId.replace(/[^a-zA-Z0-9._-]/g, "_");
      saveBlob(
        new Blob([`${JSON.stringify(artifact, null, 2)}\n`], {
          type: "application/json",
        }),
        `ironclaw-run-${filenameRunId}.json`,
      );
    } catch (error) {
      toast(
        error instanceof Error
          ? error.message
          : t("chat.fileDownloadFailed"),
        { tone: "error" },
      );
    } finally {
      setArtifactDownloading(false);
    }
  }, [artifactDownloading, t, threadId, turnRunId]);

  if (
    role === CHAT_MESSAGE_ROLES.TOOL_ACTIVITY ||
    (toolCalls && toolCalls.length > 0)
  ) {
    const activity = (toolCalls && toolCalls.length > 0)
      ? {
          id: message.id,
          toolCalls,
        }
      : message;
    return (<ToolActivity activity={activity} />);
  }

  if (role === CHAT_MESSAGE_ROLES.THINKING) {
    return (<ThinkingDisclosure content={content} />);
  }

  if (role === CHAT_MESSAGE_ROLES.IMAGE) {
    const imgs = generatedImages || [];
    return (
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          {imgs.map((img, i) =>
            img.data_url
              ? (<img key={i} src={img.data_url} className="max-h-64 rounded-lg border border-[var(--v2-panel-border)] object-cover" alt={t("chat.generatedImageAlt")} />)
              : (
                  <div key={i} className="rounded-lg border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] px-4 py-3 text-sm text-[var(--v2-text)]">
                    <div>{t("chat.generatedImageUnavailable")}</div>
                    {img.path && (<div className="mt-1 font-mono text-xs text-[var(--v2-text-muted)]">{img.path}</div>)}
                  </div>
                )
          )}
        </div>
      </div>
    );
  }

  const timeLabel = formatTimestamp(timestamp);
  const showActions =
    role === CHAT_MESSAGE_ROLES.USER ||
    (role === CHAT_MESSAGE_ROLES.ASSISTANT && !isOptimistic);
  const showArtifactAction = Boolean(
    role === CHAT_MESSAGE_ROLES.ASSISTANT &&
    message.isFinalReply === true &&
    !isOptimistic &&
    threadId &&
    turnRunId,
  );
  const isNotice = role === CHAT_MESSAGE_ROLES.SYSTEM;
  const isError = role === CHAT_MESSAGE_ROLES.ERROR;
  const bubbleWidthClass = isUser
    ? "v2-chat-readable-width"
    : isNotice
    ? "mx-auto v2-chat-readable-width"
    : isError
    ? "mr-auto v2-chat-readable-width"
    : "w-full v2-chat-readable-width";
  const contentWidthClass =
    isUser || isError ? "min-w-0 max-w-full" : "w-full min-w-0 max-w-full";
  const showRetryAction = status === "error" && onRetry;
  const showMetaRow = showActions || showRetryAction || timeLabel;
  const contentOpacityClass = isOptimistic ? "opacity-70" : "";
  const roleStyle =
    ROLE_STYLES[role as keyof typeof ROLE_STYLES] ||
    ROLE_STYLES[CHAT_MESSAGE_ROLES.ASSISTANT];

  return (
    <div
      data-testid={`msg-${role}`}
      data-final-reply={finalReplyState}
      data-failure-category={failureCategory}
      data-failure-status={failureStatus}
      className={["group flex w-full min-w-0 flex-col", isUser ? "items-end" : "items-start"].join(" ")}
    >
      <div className={["flex min-w-0 flex-col", bubbleWidthClass].join(" ")}>
        <div
          className={[
            "text-base leading-7",
            contentWidthClass,
            roleStyle,
          ].join(" ")}
        >
          {role === CHAT_MESSAGE_ROLES.ASSISTANT ||
          role === CHAT_MESSAGE_ROLES.SYSTEM ||
          role === CHAT_MESSAGE_ROLES.ERROR
            ? (<div className={contentOpacityClass}><MarkdownRenderer content={content} /></div>)
            : (<div className="v2-wrap-anywhere whitespace-pre-wrap break-words"><span className={contentOpacityClass}>{content}</span></div>)}

          {status === "error" && (
            <div className={["mt-2 flex flex-wrap items-center gap-2 text-xs text-[var(--v2-danger-text)]", contentOpacityClass].join(" ")}>
              <span>{error}</span>
            </div>
          )}

          {images && images.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-2">
              {images.map((src, i) => (<img key={i} src={src} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt={t("chat.messageAttachmentAlt")} />))}
            </div>
          )}

          {attachments && attachments.length > 0 && (
            <>
            <div className="mt-2 flex flex-col gap-1.5">
              {attachments.map((att, i) => (<AttachmentChip
                key={att.id || i}
                att={att}
                onPreview={setPreviewAttachment}
              />))}
            </div>
            <AttachmentPreviewModal
              attachment={previewAttachment}
              onClose={() => setPreviewAttachment(null)}
            />
            </>
          )}

          {role === CHAT_MESSAGE_ROLES.ASSISTANT &&
          (<ProjectFileChips
            threadId={threadId}
            content={typeof content === "string" ? content : ""}
          />)}
        </div>
      </div>

      {showMetaRow && (
        <div
          className={[
            "mt-1 flex min-h-7 w-max v2-chat-readable-width flex-nowrap items-center gap-3 px-1 text-iron-400 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100",
            isUser
              ? "self-end justify-end"
              : isNotice
              ? "self-center justify-center"
              : "self-start justify-start",
          ].join(" ")}
        >
          {timeLabel && (<time dateTime={timestamp} className="shrink-0 font-mono text-[11px] text-[var(--v2-text-muted)]">{timeLabel}</time>)}
          {(showActions || showRetryAction) && (
            <div className="flex shrink-0 items-center gap-1">
            {showActions && (
              <button
                type="button"
                onClick={copy}
                title={copied ? t("common.copied") : t("chat.copyMessage")}
                aria-label={copied ? t("common.copied") : t("chat.copyMessage")}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-[var(--v2-text-strong)]"
              >
                <Icon name={copied ? "check" : "copy"} className="h-3.5 w-3.5" />
              </button>
            )}
            {showArtifactAction && (
              <button
                type="button"
                onClick={downloadArtifact}
                disabled={artifactDownloading}
                title={artifactDownloading ? t("common.loading") : t("common.download")}
                aria-label={artifactDownloading ? t("common.loading") : t("common.download")}
                data-testid="download-run-artifact"
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-[var(--v2-text-strong)] disabled:opacity-50"
              >
                <Icon name="download" className="h-3.5 w-3.5" />
              </button>
            )}
            {showRetryAction && (
              <button
                type="button"
                onClick={() => onRetry?.(message)}
                title={t("chat.retryMessage")}
                aria-label={t("chat.retryMessage")}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 text-[var(--v2-danger-text)] hover:bg-[var(--v2-danger-soft)]"
              >
                <Icon name="retry" className="h-3.5 w-3.5" />
              </button>
            )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// Memoized: during streaming the message list re-renders on every chunk,
// but only the streaming message's `message` reference changes. Bubbles
// whose `message`/`onRetry` props are unchanged skip re-rendering (and so
// skip re-parsing their markdown). Relies on unchanged messages keeping a
// stable object identity across `setMessages` updates, and on `onRetry`
// being a stable callback from the parent.
export const MessageBubble = React.memo(MessageBubbleImpl);
