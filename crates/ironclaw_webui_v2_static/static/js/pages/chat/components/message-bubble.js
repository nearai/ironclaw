import { React, html } from "../../../lib/html.js";
import { MarkdownRenderer } from "./markdown-renderer.js";
import { ToolActivity } from "./tool-activity.js";
import { Icon } from "../../../design-system/icons.js";
import { toast } from "../../../lib/toast.js";

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
  const [open, setOpen] = React.useState(false);
  if (!content) return null;
  return html`
    <div className="flex flex-col items-start">
      <button
        type="button"
        onClick=${() => setOpen((v) => !v)}
        aria-expanded=${open ? "true" : "false"}
        className="v2-button inline-flex items-center gap-1.5 border-0 bg-transparent px-1 py-1 text-xs font-medium text-iron-400 hover:text-iron-200"
      >
        <${Icon} name="spark" className="h-3.5 w-3.5" />
        <span>${open ? "Hide reasoning" : "Reasoning"}</span>
        <${Icon}
          name="chevron"
          className=${["h-3 w-3", open ? "rotate-180" : ""].join(" ")}
        />
      </button>
      ${open &&
      html`
        <div className="mt-1 border-l-2 border-white/10 pl-3 text-iron-300">
          <${MarkdownRenderer} content=${content} className="text-[13px]" />
        </div>
      `}
    </div>
  `;
}

function MessageBubbleImpl({ message, onRetry }) {
  const { role, content, images, attachments, generatedImages, isOptimistic, status, error, toolCalls, timestamp } = message;
  const isUser = role === "user";
  const [copied, setCopied] = React.useState(false);
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
      toast("Copied to clipboard", { tone: "success" });
      setTimeout(() => setCopied(false), 1400);
    } catch {
      // clipboard unavailable — no-op
    }
  }, [content]);

  if (role === "tool_activity" || (toolCalls && toolCalls.length > 0)) {
    const activity = (toolCalls && toolCalls.length > 0)
      ? {
          id: message.id,
          toolCalls,
        }
      : message;
    return html`<${ToolActivity} activity=${activity} />`;
  }

  if (role === "thinking") {
    return html`<${ThinkingDisclosure} content=${content} />`;
  }

  if (role === "image") {
    const imgs = generatedImages || [];
    return html`
      <div className="flex">
        <div className="flex flex-wrap gap-2">
          ${imgs.map((img, i) =>
            img.data_url
              ? html`<img key=${i} src=${img.data_url} className="max-h-64 rounded-lg border border-iron-700 object-cover" alt="Generated result" />`
              : html`
                  <div key=${i} className="rounded-lg border border-iron-700 bg-iron-900/70 px-4 py-3 text-sm text-iron-200">
                    <div>Generated image unavailable in history payload</div>
                    ${img.path && html`<div className="mt-1 font-mono text-xs text-iron-300">${img.path}</div>`}
                  </div>
                `
          )}
        </div>
      </div>
    `;
  }

  const timeLabel = formatTimestamp(timestamp);
  const showActions = (role === "assistant" || role === "user") && !isOptimistic;

  return html`
    <div className=${["group flex flex-col", isUser ? "items-end" : "items-start"].join(" ")}>
      <div className="flex min-w-0 max-w-[85%] flex-col gap-1">
        <div
          className=${[
            "text-sm leading-6",
            ROLE_STYLES[role] || ROLE_STYLES.assistant,
            isOptimistic ? "opacity-70" : "",
          ].join(" ")}
        >
          ${role === "assistant" || role === "system" || role === "error"
            ? html`<${MarkdownRenderer} content=${content} />`
            : html`<div className="whitespace-pre-wrap">${content}</div>`}

          ${status === "error" && html`
            <div className="mt-2 flex flex-wrap items-center gap-2 text-xs text-red-300">
              <span>${error}</span>
            </div>
          `}

          ${images && images.length > 0 && html`
            <div className="mt-2 flex flex-wrap gap-2">
              ${images.map((src, i) => html`<img key=${i} src=${src} className="max-h-48 rounded-lg border border-iron-700 object-cover" alt="Message attachment" />`)}
            </div>
          `}

          ${attachments && attachments.length > 0 && html`
            <div className="mt-2 flex flex-col gap-1.5">
              ${attachments.map((att, i) => html`
                <div key=${i} className="flex items-center gap-2 rounded-md border border-iron-700 bg-iron-900/50 px-3 py-2 text-xs">
                  <${Icon} name="file" className="h-3.5 w-3.5 text-signal" />
                  <span className="truncate">${att.filename || "attachment"}</span>
                  <span className="ml-auto shrink-0 text-iron-200">${att.mime_type} ${att.size_label ? " / " + att.size_label : ""}</span>
                </div>
              `)}
            </div>
          `}
        </div>

        ${(showActions || status === "error" || timeLabel) && html`
          <div
            className=${[
              "flex items-center gap-1.5 px-1 text-iron-400 opacity-0 group-hover:opacity-100 focus-within:opacity-100",
              isUser ? "justify-end" : "justify-start",
            ].join(" ")}
          >
            ${showActions && html`
              <button
                type="button"
                onClick=${copy}
                aria-label="Copy message"
                className="v2-button inline-flex items-center gap-1 rounded-md border-0 bg-transparent px-1.5 py-1 text-[11px] hover:text-iron-100"
              >
                <${Icon} name=${copied ? "check" : "copy"} className="h-3.5 w-3.5" />
                ${copied ? "Copied" : "Copy"}
              </button>
            `}
            ${status === "error" && onRetry && html`
              <button
                type="button"
                onClick=${() => onRetry(message)}
                aria-label="Retry message"
                className="v2-button inline-flex items-center gap-1 rounded-md border-0 bg-transparent px-1.5 py-1 text-[11px] text-red-300 hover:text-red-200"
              >
                <${Icon} name="retry" className="h-3.5 w-3.5" />
                Retry
              </button>
            `}
            ${timeLabel && html`<span className="font-mono text-[10px] text-iron-500">${timeLabel}</span>`}
          </div>
        `}
      </div>
    </div>
  `;
}

// Memoized: during streaming the message list re-renders on every chunk,
// but only the streaming message's `message` reference changes. Bubbles
// whose `message`/`onRetry` props are unchanged skip re-rendering (and so
// skip re-parsing their markdown). Relies on unchanged messages keeping a
// stable object identity across `setMessages` updates, and on `onRetry`
// being a stable callback from the parent.
export const MessageBubble = React.memo(MessageBubbleImpl);
