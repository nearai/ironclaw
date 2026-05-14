import { Icon } from "../../../design-system/icons.js";
import { Button } from "../../../design-system/button.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import {
  formatSize,
  useComposerAttachments,
} from "../hooks/useComposerAttachments.js";

export function ChatInput({
  onSend,
  disabled,
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
  const textareaRef = React.useRef(null);
  const {
    images,
    attachments,
    addFiles,
    removeImage,
    removeAttachment,
    clearAttachments,
  } = useComposerAttachments();

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
    if (
      (!text.trim() && images.length === 0 && attachments.length === 0) ||
      disabled ||
      isSending
    )
      return;
    setIsSending(true);
    try {
      await onSend(text.trim(), { images, attachments });
      setText("");
      clearAttachments();
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    } catch {
      // The failed optimistic message renders retry details in the thread.
    } finally {
      setIsSending(false);
    }
  }, [
    text,
    images,
    attachments,
    disabled,
    isSending,
    onSend,
    clearAttachments,
  ]);

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
        addFiles(files);
      }
    },
    [addFiles]
  );

  const onDrop = React.useCallback(
    (e) => {
      e.preventDefault();
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) addFiles(files);
    },
    [addFiles]
  );

  const onDragOver = React.useCallback((e) => e.preventDefault(), []);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      if (files.length > 0) addFiles(files);
      e.target.value = "";
    },
    [addFiles]
  );

  const hasPayload =
    text.trim() || images.length > 0 || attachments.length > 0;
  const placeholder = isHero
    ? t("chat.heroPlaceholder")
    : t("chat.followUpPlaceholder");
  const shellClass = isHero
    ? "w-full"
    : "px-4 py-4 sm:px-5 lg:px-8";
  const composerClass = [
    "mx-auto w-full max-w-5xl rounded-[26px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-3",
    isHero ? "min-h-[190px]" : "min-h-[154px]",
    disabled ? "opacity-70" : "",
  ].join(" ");
  const textClass = [
    "w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6",
    "text-white outline-none placeholder:text-iron-700 focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",
    isHero ? "min-h-[96px]" : "min-h-[72px]",
  ].join(" ");

  return html`
    <div className=${shellClass}>
      <div
        className=${composerClass}
        onDrop=${onDrop}
        onDragOver=${onDragOver}
      >
        ${(images.length > 0 || attachments.length > 0) &&
        html`
          <div className="mb-3 flex flex-wrap gap-2">
            ${images.map(
              (img, i) => html`
                <div key=${i} className="group relative">
                  <img
                    src=${img.dataUrl}
                    className="h-16 w-16 rounded-lg border border-iron-700 object-cover"
                    alt=""
                  />
                  <button
                    onClick=${() => removeImage(i)}
                    className="absolute -right-1 -top-1 flex h-5 w-5 items-center justify-center rounded-full border border-red-300/30 bg-red-500 text-white opacity-0 group-hover:opacity-100"
                    aria-label=${t("chat.removeImage")}
                  >
                    <${Icon} name="close" className="h-3 w-3" />
                  </button>
                </div>
              `
            )}
            ${attachments.map(
              (att, i) => html`
                <div
                  key=${i}
                  className="flex max-w-full items-center gap-2 rounded-md border border-iron-700 bg-iron-900 px-2 py-1 text-xs"
                >
                  <${Icon} name="file" className="h-3.5 w-3.5 shrink-0 text-signal" />
                  <span className="truncate">${att.filename}</span>
                  <span className="shrink-0 text-iron-200"
                    >${formatSize(att.size)}</span
                  >
                  <button
                    onClick=${() => removeAttachment(i)}
                    className="ml-1 text-iron-200 hover:text-white"
                    aria-label=${t("chat.removeAttachment")}
                  >
                    <${Icon} name="close" className="h-3.5 w-3.5" />
                  </button>
                </div>
              `
            )}
          </div>
        `}

        <textarea
          ref=${textareaRef}
          value=${text}
          onChange=${(e) => setText(e.target.value)}
          onKeyDown=${onKeyDown}
          onPaste=${onPaste}
          placeholder=${placeholder}
          rows=${1}
          disabled=${disabled}
          className=${textClass}
        />

        <div className="mt-3 flex flex-wrap items-center gap-2">
          <label
            className="flex h-11 w-11 shrink-0 cursor-pointer items-center justify-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))] hover:text-[var(--v2-accent-text)]"
            title=${t("chat.attachFiles")}
          >
            <input
              type="file"
              multiple
              className="hidden"
              onChange=${onFileInputChange}
            />
            <${Icon} name="attach" className="h-5 w-5" />
          </label>

          <div className="ml-auto flex min-w-0 items-center gap-2">
            ${disabled &&
            html`
              <span className="hidden items-center gap-2 text-xs text-[var(--v2-text-muted)] sm:inline-flex">
                <span
                  className="h-2 w-2 rounded-full bg-[var(--v2-accent)]"
                />
                ${statusText || t("chat.statusWorking")}
              </span>
            `}
            <${ComposerPill}
              icon="bolt"
              label=${formatModelLabel(context.model, context.backend)}
              strong=${true}
            />
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
          </div>
        </div>
      </div>

      <${ComposerContextRow} context=${context} />
    </div>
  `;
}

function ComposerPill({
  icon,
  label,
  tone = "muted",
  strong = false,
  className = "",
}) {
  const toneClass =
    tone === "signal"
      ? "border-signal/35 bg-signal/10 text-signal"
      : "border-white/10 bg-white/[0.035] text-iron-300";
  return html`
    <span
      className=${[
        "inline-flex h-9 max-w-[220px] items-center gap-2 rounded-full border px-3 text-sm",
        toneClass,
        strong ? "font-semibold text-white" : "font-medium",
        className,
      ].join(" ")}
      title=${label}
    >
      <${Icon} name=${icon} className="h-4 w-4 shrink-0" />
      <span className="truncate">${label}</span>
    </span>
  `;
}

function ComposerContextRow({ context }) {
  const items = [
    context.threadLabel,
    context.turnCountLabel,
    context.engineLabel,
    context.connectionLabel,
  ].filter(Boolean);

  if (items.length === 0) return null;

  return html`
    <div
      className="mx-auto mt-3 flex w-full max-w-5xl flex-wrap items-center gap-x-4 gap-y-2 px-2 text-sm text-iron-300"
    >
      ${items.map(
        (item, index) => html`
          <span key=${`${item}-${index}`} className="inline-flex items-center gap-2">
            <span className="h-1.5 w-1.5 rounded-full bg-iron-700" />
            <span className="truncate">${item}</span>
          </span>
        `
      )}
    </div>
  `;
}

function formatModelLabel(model, backend) {
  const raw = model || backend || "";
  if (!raw) return "Model ready";
  const cleaned = String(raw).split("/").pop();
  if (cleaned.length <= 22) return cleaned;
  return `${cleaned.slice(0, 9)}...${cleaned.slice(-9)}`;
}
