import { Icon } from "../../../design-system/icons";
import { Button } from "../../../design-system/button";
import React from "react";
import { useT } from "../../../lib/i18n";
import { authScope } from "../../../lib/auth-scope";
import { stageFiles } from "../lib/attachments";
import { ATTACHMENTS_ONLY_CONTENT } from "../lib/attachment-sentinel";
import {
  INITIAL_COMMAND_MENU_SELECTION,
  commandMenuMatches,
  commandMenuSelectionReducer,
  commandMenuToken,
} from "../lib/chat-commands";
import { useAttachmentConfig } from "../hooks/useAttachmentConfig";
import {
  NEW_DRAFT_KEY,
  clearDraft,
  clearStagedAttachments,
  getDraft,
  getStagedAttachments,
  setDraft,
  setStagedAttachments,
} from "../lib/draft-store";

export function ChatInput({
  onSend,
  commands = [],
  onCancel,
  disabled,
  sendDisabled = disabled,
  canCancel = false,
  initialText = "",
  resetKey = "",
  draftKey = NEW_DRAFT_KEY,
  variant = "dock",
  context = {},
  statusText = "",
}) {
  const t = useT();
  const storageScope = authScope();
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
  const textRef = React.useRef(text);
  const currentDraftKeyRef = React.useRef(draftKey);
  currentDraftKeyRef.current = draftKey;
  const textareaRef = React.useRef(null);
  const fileInputRef = React.useRef(null);
  const sendBlockedRef = React.useRef(false);
  const sendBlocked = disabled || sendDisabled || isSending;
  const submitDisabledRef = React.useRef(disabled || sendDisabled);
  submitDisabledRef.current = disabled || sendDisabled;
  sendBlockedRef.current = sendBlocked;
  // Mirror of `attachments` plus a serial promise, so overlapping addFiles()
  // calls validate against the latest staged set rather than a stale snapshot
  // (each stageFiles is async; without this two fast drops could both admit
  // files past the per-message budget).
  const attachmentsRef = React.useRef([]);
  const stagingQueueRef = React.useRef(Promise.resolve());
  const activeDraftContextRef = React.useRef({ draftKey, storageScope });
  activeDraftContextRef.current = { draftKey, storageScope };
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
    const restored = getDraft(draftKey);
    textRef.current = restored;
    setText(restored);
    // Flush any queued write (for the previous key) before this key changes
    // or the composer unmounts, so a debounced draft is never lost. The
    // authenticated scope is part of the dependency list because the same
    // composer can stay mounted across a token/session switch.
    return () => flushDraft();
  }, [draftKey, storageScope, flushDraft]);

  // Keep the in-memory staged-attachment store in sync so files survive
  // navigating away from (and back to) this composer, the same way the text
  // draft does. On a conversation switch, *re-read* the new key's files and
  // skip persisting this render — `attachments` still belongs to the previous
  // key, so persisting it here would leak the previous conversation's files
  // into the new one.
  const stagedDraftKeyRef = React.useRef(draftKey);
  const stagedDraftScopeRef = React.useRef(storageScope);

  // Command menu: `commandToken`/`menuCommands` are pure per-render
  // derivations of `text`/`commands` (see chat-commands.ts). `menuSelection`
  // (active row + Esc-dismissed flag) is that file's small pure reducer
  // contract — this component only dispatches actions and stores the result.
  // Both the state and the derived matches are mirrored into refs for the
  // same reason `textRef` exists above: the keydown handler must read the
  // live value instead of the one captured at the last render.
  const commandToken = commandMenuToken(text);
  const menuCommands = commandMenuMatches(text, commands);
  const [menuSelection, setMenuSelection] = React.useState(
    INITIAL_COMMAND_MENU_SELECTION
  );
  const menuSelectionRef = React.useRef(menuSelection);
  menuSelectionRef.current = menuSelection;
  const menuCommandsRef = React.useRef(menuCommands);
  menuCommandsRef.current = menuCommands;
  // Tracks the token as of the last `handleChange` (or mount), so typing that
  // changes the filtered set can reset the selection — see `handleChange`.
  const commandTokenRef = React.useRef(commandToken);
  const menuOpen = menuCommands.length > 0 && !menuSelection.dismissed;
  const activeMenuIndex = Math.max(
    0,
    Math.min(menuSelection.index, menuCommands.length - 1)
  );
  const activeMenuCommand = menuOpen ? menuCommands[activeMenuIndex] : null;

  React.useEffect(() => {
    if (
      stagedDraftKeyRef.current !== draftKey ||
      stagedDraftScopeRef.current !== storageScope
    ) {
      stagedDraftKeyRef.current = draftKey;
      stagedDraftScopeRef.current = storageScope;
      setAttachments(getStagedAttachments(draftKey));
      // The composer stays mounted across conversation switches, so a stale
      // staging error would otherwise persist into every other thread or user.
      setAttachmentError("");
      return;
    }
    setStagedAttachments(draftKey, attachments);
  }, [draftKey, storageScope, attachments]);

  React.useEffect(() => {
    if (!initialText) return;
    textRef.current = initialText;
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
      const expectedDraftKey = draftKey;
      const expectedStorageScope = storageScope;
      // Chain on the staging queue so calls run one-at-a-time and each sees the
      // attachments admitted by the previous one (via attachmentsRef). The
      // `.catch` guarantees the shared queue promise always resolves — an
      // unexpected staging failure must not permanently reject it and skip every
      // later add.
      stagingQueueRef.current = stagingQueueRef.current
        .then(async () => {
          const expectedDraftKey = draftKey;
          const expectedStorageScope = storageScope;
          const { staged, errors } = await stageFiles(files, {
            limits,
            existing: attachmentsRef.current,
            t,
          });
          const current = activeDraftContextRef.current;
          if (
            current.draftKey !== expectedDraftKey ||
            current.storageScope !== expectedStorageScope ||
            authScope() !== expectedStorageScope
          ) {
            return;
          }
          if (staged.length > 0) {
            const next = [...attachmentsRef.current, ...staged];
            attachmentsRef.current = next;
            setStagedAttachments(expectedDraftKey, next);
            setAttachments(next);
          }
          setAttachmentError(errors.length > 0 ? errors.join(" ") : "");
        })
        .catch(() => {
          setAttachmentError(t("chat.attachmentStagingFailed"));
        });
    },
    [disabled, draftKey, limits, storageScope, t]
  );

  const removeAttachment = React.useCallback((id) => {
    const next = attachmentsRef.current.filter((att) => att.id !== id);
    // Keep both the ref and draft store in lockstep so a same-tick add or
    // composer remount observes the post-removal set.
    attachmentsRef.current = next;
    setStagedAttachments(draftKey, next);
    setAttachments(next);
    setAttachmentError("");
  }, [draftKey]);

  const openFilePicker = React.useCallback(() => {
    if (disabled) return;
    fileInputRef.current?.click();
  }, [disabled]);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      addFiles(files);
      // Reset so picking the same file again re-fires `change`.
      e.currentTarget.value = "";
    },
    [addFiles]
  );

  const handleSend = React.useCallback(async () => {
    // Read the live refs instead of the values captured by the last render.
    // A keydown can follow an input or async attachment update before React
    // commits another render; using captured state in that window silently
    // treats the new payload as empty and drops the Enter submit.
    const submittedText = textRef.current.trim();
    const submittedAttachments = attachmentsRef.current;
    const sendContent =
      submittedText ||
      (submittedAttachments.length > 0 ? ATTACHMENTS_ONLY_CONTENT : "");
    const submittedDraftKey = draftKey;
    const submittedScope = storageScope;
    const domSendDisabled =
      textareaRef.current?.dataset?.sendDisabled === "true";
    if (
      !sendContent ||
      disabled ||
      sendDisabled ||
      isSending ||
      domSendDisabled ||
      sendBlockedRef.current
    ) {
      return;
    }
    sendBlockedRef.current = true;
    setIsSending(true);
    textRef.current = "";
    setText("");
    setAttachments([]);
    attachmentsRef.current = [];
    setAttachmentError("");
    cancelPendingDraft();
    clearDraft(draftKey);
    clearStagedAttachments(draftKey);
    if (textareaRef.current) textareaRef.current.style.height = "auto";
    const restoreSubmittedDraft = () => {
      const scopeUnchanged = authScope() === submittedScope;
      const draftKeyUnchanged = currentDraftKeyRef.current === submittedDraftKey;
      if (!scopeUnchanged) return;
      const shouldRestoreActiveText = draftKeyUnchanged && textRef.current === "";
      if (shouldRestoreActiveText) {
        textRef.current = submittedText;
        setText(submittedText);
      }
      if (shouldRestoreActiveText || !draftKeyUnchanged) {
        setDraft(submittedDraftKey, submittedText);
      }
      const shouldRestoreActiveAttachments =
        draftKeyUnchanged &&
        attachmentsRef.current.length === 0 &&
        submittedAttachments.length > 0;
      if (shouldRestoreActiveAttachments) {
        setAttachments(submittedAttachments);
        attachmentsRef.current = submittedAttachments;
      }
      if (
        (shouldRestoreActiveAttachments || !draftKeyUnchanged) &&
        submittedAttachments.length > 0
      ) {
        setStagedAttachments(submittedDraftKey, submittedAttachments);
      }
    };
    try {
      const response = await onSend(sendContent, {
        attachments: submittedAttachments,
        displayContent: submittedText,
      });
      if (response === null) restoreSubmittedDraft();
    } catch {
      // The failed optimistic message renders retry details in the thread.
      restoreSubmittedDraft();
    } finally {
      sendBlockedRef.current = submitDisabledRef.current;
      setIsSending(false);
    }
  }, [
    disabled,
    sendDisabled,
    isSending,
    onSend,
    draftKey,
    storageScope,
    cancelPendingDraft,
  ]);

  const handleChange = React.useCallback(
    (e) => {
      const next = e.currentTarget.value;
      textRef.current = next;
      setText(next);
      // Re-filtering (the command token changed) drops any stale row
      // selection and un-suppresses a menu the user Esc-dismissed for a
      // different prefix — see the "reset" case in chat-commands.ts.
      const nextCommandToken = commandMenuToken(next);
      if (nextCommandToken !== commandTokenRef.current) {
        commandTokenRef.current = nextCommandToken;
        const resetSelection = commandMenuSelectionReducer(
          menuSelectionRef.current,
          { type: "reset" }
        );
        menuSelectionRef.current = resetSelection;
        setMenuSelection(resetSelection);
      }
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

  // Complete the draft to the full command word — shared by keyboard
  // (Enter/Tab) and mouse (click) completion so both paths stay in lockstep.
  const completeMenuCommand = React.useCallback((command) => {
    if (!command) return;
    const next = `/${command.name} `;
    textRef.current = next;
    setText(next);
    textareaRef.current?.focus();
  }, []);

  // Hover selects a row without completing it (click still completes).
  const selectMenuIndex = React.useCallback((index) => {
    const next = commandMenuSelectionReducer(menuSelectionRef.current, {
      type: "select",
      index,
    });
    menuSelectionRef.current = next;
    setMenuSelection(next);
  }, []);

  const onKeyDown = React.useCallback(
    (e) => {
      // Layer the command-menu's own keyboard handling before the
      // Enter-to-send path below, but only while the menu is actually open —
      // read live refs (not the `menuOpen`/`menuCommands` closed over at the
      // last render) so a keystroke right after typing still sees the
      // current matches.
      const openMenuCommands = menuCommandsRef.current;
      const menuIsOpen =
        openMenuCommands.length > 0 && !menuSelectionRef.current.dismissed;
      if (menuIsOpen) {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          const delta = e.key === "ArrowDown" ? 1 : -1;
          const next = commandMenuSelectionReducer(menuSelectionRef.current, {
            type: "move",
            delta,
            count: openMenuCommands.length,
          });
          menuSelectionRef.current = next;
          setMenuSelection(next);
          return;
        }
        if ((e.key === "Enter" || e.key === "Tab") && !e.shiftKey) {
          e.preventDefault();
          const boundedIndex = Math.max(
            0,
            Math.min(menuSelectionRef.current.index, openMenuCommands.length - 1)
          );
          completeMenuCommand(openMenuCommands[boundedIndex]);
          return;
        }
        // Shift+Enter/Shift+Tab fall through unhandled here — identical to
        // the menu-closed case (native newline / focus-shift), not a
        // completion and not a send.
        if (e.key === "Escape") {
          e.preventDefault();
          const next = commandMenuSelectionReducer(menuSelectionRef.current, {
            type: "dismiss",
          });
          menuSelectionRef.current = next;
          setMenuSelection(next);
          return;
        }
      }
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        const domSendDisabled =
          e.currentTarget?.dataset?.sendDisabled === "true" ||
          textareaRef.current?.dataset?.sendDisabled === "true";
        if (domSendDisabled || sendBlockedRef.current) return;
        handleSend();
      }
    },
    [handleSend, completeMenuCommand]
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

  const hasPayload = text.trim() || attachments.length > 0;
  const isSubmitDisabled = disabled || sendDisabled;
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
    // Suppressed only when the composer is hard-disabled; busy runs
    // still allow draft editing.
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

  return (
    <div className={shellClass}>
      <div
        className={composerClass}
        onDrop={onDrop}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
      >
        {dragOver &&
        (
          <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
            {t("chat.attachmentDropHint")}
          </div>
        )}
        {menuOpen &&
        (
          // Anchored above the composer (not in normal flow) so the menu
          // floats over the canvas instead of shoving the send button down
          // as rows come and go while typing.
          <div
            id="chat-command-menu-listbox"
            role="listbox"
            aria-label={t("chat.commandMenu")}
            className="absolute bottom-full left-0 right-0 z-20 mb-2 max-h-64 overflow-y-auto rounded-md border border-iron-700 bg-iron-900/95 text-xs shadow-[0_18px_40px_-18px_rgba(0,0,0,0.7)]"
          >
            {menuCommands.map(
              (command, index) => {
                const isActive = index === activeMenuIndex;
                const prefixLength = Math.min(commandToken.length, command.name.length);
                const matchedPrefix = command.name.slice(0, prefixLength);
                const restOfName = command.name.slice(prefixLength);
                return (
                  <button
                    key={command.name}
                    id={`chat-command-option-${command.name}`}
                    type="button"
                    role="option"
                    aria-selected={isActive}
                    onMouseDown={(e) => e.preventDefault()}
                    onMouseEnter={() => selectMenuIndex(index)}
                    onClick={() => completeMenuCommand(command)}
                    className={[
                      "flex w-full flex-col gap-0.5 px-3 py-1.5 text-left",
                      isActive ? "bg-iron-800" : "hover:bg-iron-800",
                    ].join(" ")}
                  >
                    <span className="flex min-w-0 items-baseline gap-2">
                      <span className="shrink-0 font-mono text-iron-100">
                        /<span className="text-signal">{matchedPrefix}</span>{restOfName}
                      </span>
                      <span className="shrink-0 font-medium text-iron-100">{command.title}</span>
                      <span className="min-w-0 truncate text-iron-400">{command.description}</span>
                    </span>
                    {isActive &&
                    (<span className="text-iron-400">{command.usage}</span>)}
                  </button>
                );
              }
            )}
          </div>
        )}
        {attachmentError &&
        (
          <div
            role="alert"
            className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
          >
            <span className="min-w-0 flex-1">{attachmentError}</span>
            <button
              type="button"
              onClick={() => setAttachmentError("")}
              aria-label={t("common.dismiss")}
              title={t("common.dismiss")}
              className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
            >
              <Icon name="close" className="h-3.5 w-3.5" strokeWidth={2} />
            </button>
          </div>
        )}

        {attachments.length > 0 &&
        (
          <div className="mb-2 flex flex-wrap gap-2 px-1">
            {attachments.map(
              (att) => (
                <div
                  key={att.id}
                  className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
                >
                  {att.previewUrl
                    ? (<img
                        src={att.previewUrl}
                        alt={att.filename}
                        className="h-9 w-9 shrink-0 rounded object-cover"
                      />)
                    : (<span
                        className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                      >
                        <Icon name="file" className="h-4 w-4" />
                      </span>)}
                  <span className="flex min-w-0 flex-col">
                    <span className="max-w-[12rem] truncate font-medium">
                      {att.filename}
                    </span>
                    <span className="text-[10px] text-iron-400">{att.sizeLabel}</span>
                  </span>
                  <button
                    type="button"
                    onClick={() => removeAttachment(att.id)}
                    aria-label={t("chat.attachmentRemove")}
                    title={t("chat.attachmentRemove")}
                    className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                  >
                    <Icon name="close" className="h-3 w-3" />
                  </button>
                </div>
              )
            )}
          </div>
        )}

        <textarea
          ref={textareaRef}
          data-testid="chat-composer"
          value={text}
          onChange={handleChange}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          data-send-disabled={isSubmitDisabled ? "true" : "false"}
          placeholder={placeholder}
          rows={1}
          disabled={disabled}
          aria-expanded={menuOpen}
          aria-controls={menuOpen ? "chat-command-menu-listbox" : undefined}
          aria-activedescendant={
            activeMenuCommand
              ? `chat-command-option-${activeMenuCommand.name}`
              : undefined
          }
          className={textClass}
        />

        <input
          ref={fileInputRef}
          type="file"
          multiple
          accept={acceptAttr}
          className="hidden"
          onChange={onFileInputChange}
        />

        <div className="mt-2 flex items-center gap-2">
          {isSubmitDisabled &&
          (
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              {statusText || t("chat.statusWorking")}
            </span>
          )}
          <div className="ml-auto flex items-center gap-1.5">
            <button
              type="button"
              onClick={openFilePicker}
              disabled={disabled}
              aria-label={t("chat.attachFiles")}
              title={t("chat.attachFiles")}
              className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
            >
              <Icon name="plus" className="h-5 w-5" />
            </button>
            {canCancel
              ? (
                <Button
                  type="button"
                  variant="danger"
                  size="icon-sm"
                  data-testid="chat-cancel-run"
                  onClick={handleCancel}
                  disabled={isCancelling}
                  aria-label={t("common.cancel")}
                  title={t("common.cancel")}
                  className="rounded-full"
                >
                  <Icon name="close" className="h-5 w-5" />
                </Button>
              )
              : (
                <Button
                  type="button"
                  variant="primary"
                  size="icon-sm"
                  onClick={handleSend}
                  disabled={isSubmitDisabled || isSending || !hasPayload}
                  aria-label={t("chat.send")}
                  className="rounded-full"
                >
                  <Icon name="send" className="h-5 w-5" />
                </Button>
              )}
          </div>
        </div>
      </div>
    </div>
  );
}
