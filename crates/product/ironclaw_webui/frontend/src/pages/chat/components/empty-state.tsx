import React from "react";
import { useOobeSuggestionsEnabled } from "../../../app/auth";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { ChatInput } from "./chat-input";
import { NearProcessIndicator } from "./near-process-indicator";

// The OOBE suggestion surface is gated off by default (see
// suggested-task-surface.tsx), and even when it renders, its cards/icons/
// NearProcessIndicator import weight has no business padding every /chat
// page load — so it loads as its own chunk instead, the same pattern
// message-bubble.tsx uses for CommandResult/AttachmentPreviewModal.
const SuggestedTaskSurface = React.lazy(() =>
  import("./suggested-task-surface").then(({ SuggestedTaskSurface }) => ({
    default: SuggestedTaskSurface,
  }))
);

// The restore pill only appears after the (lazy) surface has loaded and been
// dismissed, so its markup is lazy too — keeping OOBE weight out of eager /chat.
const OobeRestorePill = React.lazy(() =>
  import("./oobe-restore-pill").then(({ OobeRestorePill }) => ({
    default: OobeRestorePill,
  }))
);

// Passed down to SuggestedTaskSurface -> SuggestedTaskCard as a render prop so
// the lazy-loaded surface/card chunk doesn't need its own import of
// NearProcessIndicator, which is already eager-reachable via
// typing-indicator.tsx -> message-list.tsx -> chat.tsx. Importing it from both
// an eager path and the lazy chunk would force the bundler to split it into
// its own small standalone chunk instead of keeping it inlined where it
// already lives. Module scope (no closure over component state) so it's a
// stable reference across renders.
function renderRunningIndicator(label: string) {
  return <NearProcessIndicator state="working" label={label} />;
}

export function EmptyState({
  onSuggestion,
  onSend,
  onOpenThread,
  commands = [],
  disabled,
  sendDisabled,
  initialText,
  resetKey,
  draftKey,
  context,
  statusText,
  canCancel,
  onCancel,
}) {
  const t = useT();
  const oobeSuggestionsEnabled = useOobeSuggestionsEnabled();
  // Section-level drawer visibility (distinct from per-card dismiss, which the
  // surface owns): "open" shows the drawer; "dismissed" hides it and shows the
  // in-composer "Show suggestions" pill to restore it; "gone" hides both.
  const [drawerState, setDrawerState] = React.useState<
    "open" | "dismissed" | "gone"
  >("open");
  const showRestorePill = oobeSuggestionsEnabled && drawerState === "dismissed";
  const suggestions = [
    {
      icon: "tool",
      title: t("chat.suggestion1"),
      detail: t("chat.suggestion1Desc"),
    },
    {
      icon: "shield",
      title: t("chat.suggestion2"),
      detail: t("chat.suggestion2Desc"),
    },
    {
      icon: "plug",
      title: t("chat.suggestion3"),
      detail: t("chat.suggestion3Desc"),
    },
  ];

  return (
    <div
      className="v2-page-entrance flex min-h-0 flex-1 flex-col items-center justify-center px-4 py-8 sm:px-8 lg:px-12"
    >
      <div className="w-full max-w-5xl text-center">
        <h2
          className="mx-auto max-w-[16ch] text-4xl font-semibold leading-[1.04] text-white sm:text-5xl lg:text-6xl"
        >
          {t("chat.heroTitle")}
        </h2>
        <p
          className="mx-auto mt-4 max-w-[64ch] text-base leading-relaxed text-iron-300"
        >
          {t("chat.heroDesc")}
        </p>
      </div>

      {oobeSuggestionsEnabled && (
        <React.Suspense fallback={null}>
          <SuggestedTaskSurface
            onOpenThread={onOpenThread}
            renderRunningIndicator={renderRunningIndicator}
            hidden={drawerState !== "open"}
            onClose={() => setDrawerState("dismissed")}
          />
        </React.Suspense>
      )}

      <div className={`relative ${oobeSuggestionsEnabled ? "mt-3" : "mt-9"} w-full max-w-5xl`}>
        <ChatInput
          onSend={onSend}
          commands={commands}
          disabled={disabled}
          sendDisabled={sendDisabled}
          initialText={initialText}
          resetKey={resetKey}
          draftKey={draftKey}
          variant="hero"
          context={context}
          statusText={statusText}
          canCancel={canCancel}
          onCancel={onCancel}
        />
        {/* Restore pill: shown inside the composer once the drawer is dismissed.
            Clicking the label reopens the drawer; the × dismisses it fully.
            Lazy so its markup stays out of eager /chat. */}
        {showRestorePill && (
          <React.Suspense fallback={null}>
            <OobeRestorePill
              onRestore={() => setDrawerState("open")}
              onDismiss={() => setDrawerState("gone")}
            />
          </React.Suspense>
        )}
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        {suggestions.map(
          (item) => (
            <button
              type="button"
              key={item.title}
              onClick={() => onSuggestion(item.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-white/10 px-2 py-4 text-left hover:border-signal/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal"
              >
                <Icon name={item.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-semibold text-iron-100">
                  {item.title}
                </span>
                <span className="mt-0.5 block text-sm text-iron-300">
                  {item.detail}
                </span>
              </span>
            </button>
          )
        )}
      </div>
    </div>
  );
}
