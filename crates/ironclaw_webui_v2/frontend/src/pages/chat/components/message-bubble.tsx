import React from "react";
import { MarkdownRenderer } from "./markdown-renderer";
import { ToolActivity } from "./tool-activity";
import { Icon } from "../../../design-system/icons";
import { toast } from "../../../lib/toast";
import { ProjectFileChips } from "./project-file-chips";
import { AttachmentChip } from "./attachment-chip";
import { AttachmentPreviewModal } from "./attachment-preview";
import { useT } from "../../../lib/i18n";

/* User keeps a tinted bubble; assistant is borderless (document-like);
   system / error stay as centered tinted notices. Reasoning ("thinking")
   renders as a collapsible disclosure (see ThinkingDisclosure). */
const ROLE_STYLES = {
  user: "ml-auto rounded-[18px] border border-signal/25 bg-signal/10 px-4 py-3 text-iron-100",
  assistant: "mr-auto px-1 text-iron-100",
  system: "mx-auto rounded-[18px] border border-copper/20 bg-copper/10 px-4 py-3 text-center text-copper",
  error: "mx-auto rounded-[18px] border border-red-400/20 bg-red-500/10 px-4 py-3 text-center text-red-200",
};

function formatTimestamp(value) {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/* Collapsible provider-reasoning summary. Collapsed by default so the
   thread stays clean; expands to the full reasoning markdown. Data comes
   from the `thinking` projection item (PR #4230). */
function ThinkingDisclosure({ content }) {
  const t = useT();
  const [open, setOpen] = React.useState(false);
  if (!content) return null;
  return (
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open ? "true" : "false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-iron-400 hover:text-iron-200"
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
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <MarkdownRenderer content={content} className="text-[13px]" />
        </div>
      )}
    </div>
  );
}

function MessageBubbleImpl({ message, onRetry, threadId }) {
  const t = useT();
  const { role, content, images, attachments, generatedImages, isOptimistic, status, error, toolCalls, timestamp } = message;
  const isUser = role === "user";
  const finalReplyState =
    role === "assistant" && typeof message.isFinalReply === "boolean"
      ? String(message.isFinalReply)
      : undefined;
  const [copied, setCopied] = React.useState(false);
  // The attachment currently open in the preview modal (null when closed).
  const [previewAttachment, setPreviewAttachment] = React.useState(null);
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

  if (role === "tool_activity" || (toolCalls && toolCalls.length > 0)) {
    const activity = (toolCalls && toolCalls.length > 0)
      ? {
          id: message.id,
          toolCalls,
        }
      : message;
    return (<ToolActivity activity={activity} />);
  }

  if (role === "thinking") {
    return (<ThinkingDisclosure content={content} />);
  }

  if (role === "image") {
    const imgs = generatedImages || [];
    return (
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          {imgs.map((img, i) =>
            img.data_url
              ? (<img key={i} src={img.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt={t("chat.generatedImageAlt")} />)
              : (
                  <div key={i} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>{t("chat.generatedImageUnavailable")}</div>
                    {img.path && (<div className="mt-1 font-mono text-xs text-iron-300">{img.path}</div>)}
                  </div>
                )
          )}
        </div>
      </div>
    );
  }

  const timeLabel = formatTimestamp(timestamp);
  const showActions = role === "user" || (role === "assistant" && !isOptimistic);
  const isNotice = role === "system" || role === "error";
  const bubbleWidthClass = isUser
    ? "v2-chat-readable-width"
    : isNotice
    ? "mx-auto v2-chat-readable-width"
    : "w-full v2-chat-readable-width";
  const contentWidthClass = isUser ? "min-w-0 max-w-full" : "w-full min-w-0 max-w-full";
  const showRetryAction = status === "error" && onRetry;
  const showMetaRow = showActions || showRetryAction || timeLabel;

  return (
    <div
      data-testid={`msg-${role}`}
      data-final-reply={finalReplyState}
      className={["group flex w-full min-w-0 flex-col", isUser ? "items-end" : "items-start"].join(" ")}
    >
      <div className={["flex min-w-0 flex-col", bubbleWidthClass].join(" ")}>
        <div
          className={[
            "text-base leading-7",
            contentWidthClass,
            ROLE_STYLES[role] || ROLE_STYLES.assistant,
            isOptimistic ? "opacity-70" : "",
          ].join(" ")}
        >
          {role === "assistant" || role === "system" || role === "error"
            ? (<MarkdownRenderer content={content} />)
            : (<div className="v2-wrap-anywhere whitespace-pre-wrap break-words">{content}</div>)}

          {status === "error" && (
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
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

          {role === "assistant" &&
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
            isUser ? "self-end justify-end" : isNotice ? "self-center justify-center" : "self-start justify-start",
          ].join(" ")}
        >
          {timeLabel && (<time dateTime={timestamp} className="shrink-0 font-mono text-[11px] text-iron-500">{timeLabel}</time>)}
          {(showActions || showRetryAction) && (
            <div className="flex shrink-0 items-center gap-1">
            {showActions && (
              <button
                type="button"
                onClick={copy}
                title={copied ? t("common.copied") : t("chat.copyMessage")}
                aria-label={copied ? t("common.copied") : t("chat.copyMessage")}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 hover:text-iron-100"
              >
                <Icon name={copied ? "check" : "copy"} className="h-3.5 w-3.5" />
              </button>
            )}
            {showRetryAction && (
              <button
                type="button"
                onClick={() => onRetry(message)}
                title={t("chat.retryMessage")}
                aria-label={t("chat.retryMessage")}
                className="v2-button inline-grid h-7 w-7 place-items-center rounded-md border-0 bg-transparent p-0 text-red-300 hover:text-red-200"
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
