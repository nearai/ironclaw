import { Icon } from "../../../design-system/icons.js";
import { Button } from "../../../design-system/button.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { authScope } from "../../../lib/auth-scope.js";
import {
  NEW_DRAFT_KEY,
  clearDraft,
  getDraft,
  setDraft,
} from "../lib/draft-store.js";

export function ChatInput({
  onSend,
  onCancel,
  disabled,
  canCancel = false,
  initialText = "",
  resetKey = "",
  draftKey = NEW_DRAFT_KEY,
  variant = "dock",
  context = {},
  statusText = "",
}) {
  const t = useT();
  const isHero = variant === "hero";
  const [text, setText] = React.useState(() => getDraft(draftKey));
  const [isSending, setIsSending] = React.useState(false);
  const [isCancelling, setIsCancelling] = React.useState(false);
  const [unsupportedPayloadError, setUnsupportedPayloadError] =
    React.useState("");
  const textareaRef = React.useRef(null);

  // Debounce draft persistence: localStorage writes are synchronous and
  // disk-backed, so writing on every keystroke can add typing latency. We
  // hold the latest {key, text, scope} and flush after a short idle, but also
  // flush immediately on unmount / thread switch so navigating away never
  // drops the last keystrokes, and cancel outright on send so a queued write
  // can't resurrect a just-sent draft.
  const pendingDraftRef = React.useRef(null);
  const draftTimerRef = React.useRef(null);
  const flushDraft = React.useCallback(() => {
    if (draftTimerRef.current) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
    const pending = pendingDraftRef.current;
    pendingDraftRef.current = null;
    // Drop the write if the authenticated identity changed since the draft
    // was queued (sign-out / 401 / token swap). Otherwise a flush triggered
    // by the unmount during auth teardown would re-persist the previous
    // user's text after the caches were purged.
    if (pending && pending.scope === authScope()) {
      setDraft(pending.key, pending.text);
    }
  }, []);
  const cancelPendingDraft = React.useCallback(() => {
    if (draftTimerRef.current) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
    pendingDraftRef.current = null;
  }, []);

  const autoResize = React.useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, []);

  React.useEffect(() => {
    autoResize();
  }, [text, autoResize]);

  // Clear the attachment warning whenever the composer is reset (a new
  // navigation / conversation switch changes resetKey). Without this the
  // banner is local state that survives the switch and only clears on
  // refresh or send. Runs independently of the initialText restore below,
  // which early-returns when there is no draft to restore.
  React.useEffect(() => {
    setUnsupportedPayloadError("");
  }, [resetKey]);

  // Restore the persisted draft when the active conversation changes
  // (draftKey switches). The initialText effect below runs after this
  // and overrides when a location.state draft was passed in, so an
  // explicit hand-off draft still wins over the stored one.
  React.useEffect(() => {
    setText(getDraft(draftKey));
    // Flush any queued write (for the previous key) before this key changes
    // or the composer unmounts, so a debounced draft is never lost.
    return () => flushDraft();
  }, [draftKey, flushDraft]);

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
    if (!text.trim() || disabled || isSending) return;
    setIsSending(true);
    try {
      await onSend(text.trim());
      setText("");
      cancelPendingDraft();
      clearDraft(draftKey);
      setUnsupportedPayloadError("");
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    } catch {
      // The failed optimistic message renders retry details in the thread.
    } finally {
      setIsSending(false);
    }
  }, [text, disabled, isSending, onSend, draftKey, cancelPendingDraft]);

  const handleChange = React.useCallback(
    (e) => {
      const next = e.target.value;
      setText(next);
      // Queue a debounced persist instead of writing on every keystroke.
      // Capture the scope so a flush after an identity change is dropped.
      pendingDraftRef.current = { key: draftKey, text: next, scope: authScope() };
      if (draftTimerRef.current) window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = window.setTimeout(flushDraft, 300);
    },
    [draftKey, flushDraft]
  );

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
      if (disabled) return;
      const files = Array.from(e.clipboardData.files);
      if (files.length > 0) {
        e.preventDefault();
        setUnsupportedPayloadError(t("chat.attachmentsUnsupported"));
      }
    },
    [t, disabled]
  );

  const onDrop = React.useCallback(
    (e) => {
      e.preventDefault();
      setDragOver(false);
      if (disabled) return;
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) {
        setUnsupportedPayloadError(t("chat.attachmentsUnsupported"));
      }
    },
    [t, disabled]
  );

  const [dragOver, setDragOver] = React.useState(false);
  const onDragOver = React.useCallback(
    (e) => {
      e.preventDefault();
      if (disabled) return;
      setDragOver(true);
    },
    [disabled]
  );
  const onDragLeave = React.useCallback((e) => {
    if (e.currentTarget.contains(e.relatedTarget)) return;
    setDragOver(false);
  }, []);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      if (!disabled && files.length > 0) {
        setUnsupportedPayloadError(t("chat.attachmentsUnsupported"));
      }
      e.target.value = "";
    },
    [t, disabled]
  );

  const hasPayload = text.trim();
  const placeholder = isHero
    ? t("chat.heroPlaceholder")
    : t("chat.followUpPlaceholder");
  const shellClass = isHero
    ? "w-full"
    : "px-4 py-3 sm:px-5 lg:px-8";
  const composerClass = [
    "relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",
    // Highlight the full rounded container on focus (not just the
    // leaking textarea ring), mirroring the global input:focus accent.
    // Suppressed while disabled so the Working-state composer never
    // looks interactive.
    disabled
      ? ""
      : "focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",
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
          <div className="mb-3 flex items-start gap-2 rounded-md border border-red-400/25 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-100">
            <span className="min-w-0 flex-1">${unsupportedPayloadError}</span>
            <button
              type="button"
              onClick=${() => setUnsupportedPayloadError("")}
              aria-label=${t("common.dismiss")}
              title=${t("common.dismiss")}
              className="shrink-0 rounded p-0.5 text-red-200 hover:text-red-50"
            >
              <${Icon} name="close" className="h-3.5 w-3.5" />
            </button>
          </div>
        `}

        <textarea
          ref=${textareaRef}
          data-testid="chat-composer"
          value=${text}
          onChange=${handleChange}
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
              className=${[
                "flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)]",
                disabled
                  ? "cursor-not-allowed opacity-50 pointer-events-none"
                  : "cursor-pointer hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)]",
              ].join(" ")}
              title=${t("chat.attachmentsUnsupported")}
            >
              <input
                type="file"
                multiple
                className="hidden"
                disabled=${disabled}
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
