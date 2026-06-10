import { Icon } from "../../../design-system/icons.js";
import { Button } from "../../../design-system/button.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

export function ChatInput({
  onSend,
  onCancel,
  disabled,
  canCancel = false,
  initialText = "",
  resetKey = "",
  variant = "dock",
  context = {},
  statusText = "",
}) {
  const t = useT();
  const isHero = variant === "hero";
  const [text, setText] = React.useState("");
  const [isSending, setIsSending] = React.useState(false);
  const [isCancelling, setIsCancelling] = React.useState(false);
  const [unsupportedPayloadError, setUnsupportedPayloadError] =
    React.useState("");
  const textareaRef = React.useRef(null);

  const autoResize = React.useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, []);

  React.useEffect(() => {
    autoResize();
  }, [text, autoResize]);

  React.useEffect(() => {
    if (!initialText) return;
    setText(initialText);
    window.requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.setSelectionRange(
          initialText.length,
          initialText.length
        );
      }
    });
  }, [initialText, resetKey]);

  const handleSend = React.useCallback(async () => {
    if (!text.trim() || disabled || isSending)
      return;
    setIsSending(true);
    try {
      await onSend(text.trim());
      setText("");
      setUnsupportedPayloadError("");
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    } catch {
      // The failed optimistic message renders retry details in the thread.
    } finally {
      setIsSending(false);
    }
  }, [
    text,
    disabled,
    isSending,
    onSend,
  ]);

  const handleCancel = React.useCallback(async () => {
    if (!canCancel || isCancelling || !onCancel) return;
    setIsCancelling(true);
    try {
      await onCancel();
    } finally {
      setIsCancelling(false);
    }
  }, [canCancel, isCancelling, onCancel]);

  const onKeyDown = React.useCallback(
    (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  const onPaste = React.useCallback(
    (e) => {
      const files = Array.from(e.clipboardData.files);
      if (files.length > 0) {
        e.preventDefault();
        setUnsupportedPayloadError(t("chat.attachmentsUnsupported"));
      }
    },
    [t]
  );

  const onDrop = React.useCallback(
    (e) => {
      e.preventDefault();
      setDragOver(false);
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) {
        setUnsupportedPayloadError(t("chat.attachmentsUnsupported"));
      }
    },
    [t]
  );

  const [dragOver, setDragOver] = React.useState(false);
  const onDragOver = React.useCallback((e) => {
    e.preventDefault();
    setDragOver(true);
  }, []);
  const onDragLeave = React.useCallback((e) => {
    if (e.currentTarget.contains(e.relatedTarget)) return;
    setDragOver(false);
  }, []);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      if (files.length > 0) {
        setUnsupportedPayloadError(t("chat.attachmentsUnsupported"));
      }
      e.target.value = "";
    },
    [t]
  );

  const hasPayload = text.trim();
  const placeholder = isHero
    ? t("chat.heroPlaceholder")
    : t("chat.followUpPlaceholder");
  const shellClass = isHero
    ? "w-full"
    : "px-4 py-3 sm:px-5 lg:px-8";
  const composerClass = [
    "relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5",
    isHero ? "min-h-[120px]" : "",
    disabled ? "opacity-70" : "",
  ].join(" ");
  const textClass = [
    "w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6",
    "text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",
    isHero ? "min-h-[72px]" : "min-h-[40px]",
  ].join(" ");

  return html`
    <div className=${shellClass}>
      <div
        className=${composerClass}
        onDrop=${onDrop}
        onDragOver=${onDragOver}
        onDragLeave=${onDragLeave}
      >
        ${dragOver &&
        html`
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            ${t("chat.attachmentsUnsupported")}
          </div>
        `}
        ${unsupportedPayloadError &&
        html`
          <div className="mb-3 rounded-md border border-red-400/25 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-100">
            ${unsupportedPayloadError}
          </div>
        `}

        <textarea
          ref=${textareaRef}
          data-testid="chat-composer"
          value=${text}
          onChange=${(e) => setText(e.target.value)}
          onKeyDown=${onKeyDown}
          onPaste=${onPaste}
          placeholder=${placeholder}
          rows=${1}
          disabled=${disabled}
          className=${textClass}
        />

        <div className="mt-2 flex items-center gap-2">
          ${disabled &&
          html`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${statusText || t("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            <label
              className="flex h-9 w-9 shrink-0 cursor-pointer items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)]"
              title=${t("chat.attachmentsUnsupported")}
            >
              <input
                type="file"
                multiple
                className="hidden"
                onChange=${onFileInputChange}
              />
              <${Icon} name="attach" className="h-5 w-5" />
            </label>
            ${canCancel
              ? html`
                <${Button}
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  onClick=${handleCancel}
                  disabled=${isCancelling}
                  aria-label=${t("common.cancel")}
                  title=${t("common.cancel")}
                  className="rounded-full"
                >
                  <${Icon} name="close" className="h-5 w-5" />
                <//>
              `
              : html`
                <${Button}
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick=${handleSend}
                  disabled=${disabled || isSending || !hasPayload}
                  aria-label=${t("chat.send")}
                  className="rounded-full"
                >
                  <${Icon} name="send" className="h-5 w-5" />
                <//>
              `}
          </div>
        </div>
      </div>
    </div>
  `;
}
