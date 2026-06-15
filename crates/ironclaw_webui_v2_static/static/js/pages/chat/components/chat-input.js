import { Icon } from "../../../design-system/icons.js";
import { Button } from "../../../design-system/button.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { authScope } from "../../../lib/auth-scope.js";
import { stageFiles } from "../lib/attachments.js";
import { useAttachmentConfig } from "../hooks/useAttachmentConfig.js";
import {
  NEW_DRAFT_KEY,
  clearDraft,
  clearStagedAttachments,
  getDraft,
  getStagedAttachments,
  setDraft,
  setStagedAttachments,
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
  const limits = useAttachmentConfig();
  const [text, setText] = React.useState(() => getDraft(draftKey));
  const [attachments, setAttachments] = React.useState(() =>
    getStagedAttachments(draftKey)
  );
  const [attachmentError, setAttachmentError] = React.useState("");
  const [isSending, setIsSending] = React.useState(false);
  const [isCancelling, setIsCancelling] = React.useState(false);
  const [dragOver, setDragOver] = React.useState(false);
  const textareaRef = React.useRef(null);
  const fileInputRef = React.useRef(null);
  // Mirror of `attachments` plus a serial promise, so overlapping addFiles()
  // calls validate against the latest staged set rather than a stale snapshot
  // (each stageFiles is async; without this two fast drops could both admit
  // files past the per-message budget).
  const attachmentsRef = React.useRef([]);
  const stagingQueueRef = React.useRef(Promise.resolve());
  React.useEffect(() => {
    attachmentsRef.current = attachments;
  }, [attachments]);

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

  // Keep the in-memory staged-attachment store in sync so files survive
  // navigating away from (and back to) this composer, the same way the text
  // draft does. On a conversation switch, *re-read* the new key's files and
  // skip persisting this render — `attachments` still belongs to the previous
  // key, so persisting it here would leak the previous conversation's files
  // into the new one.
  const stagedDraftKeyRef = React.useRef(draftKey);
  React.useEffect(() => {
    if (stagedDraftKeyRef.current !== draftKey) {
      stagedDraftKeyRef.current = draftKey;
      setAttachments(getStagedAttachments(draftKey));
      return;
    }
    setStagedAttachments(draftKey, attachments);
  }, [draftKey, attachments]);

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

  // Stage dropped/picked/pasted files: validate against the server contract,
  // append the accepted ones, and surface any rejection reasons as a single
  // combined notice. `stageFiles` reads bytes to base64 off the main file
  // list, so this is async.
  const addFiles = React.useCallback(
    (files) => {
      // Paste/drop can call this while the composer is disabled; don't stage then.
      if (disabled || !files || files.length === 0) return;
      // Chain on the staging queue so calls run one-at-a-time and each sees the
      // attachments admitted by the previous one (via attachmentsRef). The
      // `.catch` guarantees the shared queue promise always resolves — an
      // unexpected staging failure must not permanently reject it and skip every
      // later add.
      stagingQueueRef.current = stagingQueueRef.current
        .then(async () => {
          const { staged, errors } = await stageFiles(files, {
            limits,
            existing: attachmentsRef.current,
            t,
          });
          if (staged.length > 0) {
            setAttachments((prev) => {
              const next = [...prev, ...staged];
              attachmentsRef.current = next;
              return next;
            });
          }
          setAttachmentError(errors.length > 0 ? errors.join(" ") : "");
        })
        .catch(() => {
          setAttachmentError(t("chat.attachmentStagingFailed"));
        });
    },
    [disabled, limits, t]
  );

  const removeAttachment = React.useCallback((id) => {
    setAttachments((prev) => {
      const next = prev.filter((att) => att.id !== id);
      // Keep the ref in lockstep so a same-tick add validates against the
      // post-removal set, not a stale snapshot (the effect sync is async).
      attachmentsRef.current = next;
      return next;
    });
    setAttachmentError("");
  }, []);

  const openFilePicker = React.useCallback(() => {
    if (disabled) return;
    fileInputRef.current?.click();
  }, [disabled]);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      addFiles(files);
      // Reset so picking the same file again re-fires `change`.
      e.target.value = "";
    },
    [addFiles]
  );

  const handleSend = React.useCallback(async () => {
    // The v2 send contract requires non-empty content, so attachments
    // ride along with text rather than sending on their own.
    if (!text.trim() || disabled || isSending) return;
    setIsSending(true);
    try {
      await onSend(text.trim(), { attachments });
      setText("");
      setAttachments([]);
      attachmentsRef.current = [];
      setAttachmentError("");
      cancelPendingDraft();
      clearDraft(draftKey);
      clearStagedAttachments(draftKey);
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    } catch {
      // The failed optimistic message renders retry details in the thread.
    } finally {
      setIsSending(false);
    }
  }, [
    text,
    attachments,
    disabled,
    isSending,
    onSend,
    draftKey,
    cancelPendingDraft,
  ]);

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
      const files = Array.from(e.clipboardData?.files || []);
      if (files.length > 0) {
        e.preventDefault();
        addFiles(files);
      }
    },
    [addFiles]
  );

  const onDrop = React.useCallback(
    (e) => {
      e.preventDefault();
      setDragOver(false);
      const files = Array.from(e.dataTransfer?.files || []);
      if (files.length > 0) addFiles(files);
    },
    [addFiles]
  );

  const onDragOver = React.useCallback(
    (e) => {
      e.preventDefault();
      // `addFiles` no-ops while disabled, so don't tease the drop overlay then.
      if (disabled) return;
      setDragOver(true);
    },
    [disabled]
  );
  const onDragLeave = React.useCallback((e) => {
    if (e.currentTarget.contains(e.relatedTarget)) return;
    setDragOver(false);
  }, []);

  const hasPayload = text.trim();
  const placeholder = isHero
    ? t("chat.heroPlaceholder")
    : t("chat.followUpPlaceholder");
  const acceptAttr = limits.accept.length > 0 ? limits.accept.join(",") : undefined;
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
            ${t("chat.attachmentDropHint")}
          </div>
        `}
        ${attachmentError &&
        html`
          <div className="mb-3 rounded-md border border-red-400/25 bg-red-500/10 px-3 py-2 text-xs leading-5 text-red-100">
            ${attachmentError}
          </div>
        `}

        ${attachments.length > 0 &&
        html`
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            ${attachments.map(
              (att) => html`
                <div
                  key=${att.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  ${att.previewUrl
                    ? html`<img
                        src=${att.previewUrl}
                        alt=${att.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />`
                    : html`<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <${Icon} name="file" className="h-4 w-4" />
                      </span>`}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      ${att.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">${att.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick=${() => removeAttachment(att.id)}
                    aria-label=${t("chat.attachmentRemove")}
                    title=${t("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <${Icon} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `
            )}
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

        <input
          ref=${fileInputRef}
          type="file"
          multiple
          accept=${acceptAttr}
          className="hidden"
          onChange=${onFileInputChange}
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
            <button
              type="button"
              onClick=${openFilePicker}
              disabled=${disabled}
              aria-label=${t("chat.attachFiles")}
              title=${t("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <${Icon} name="plus" className="h-5 w-5" />
            </button>
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
