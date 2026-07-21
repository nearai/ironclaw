import { Icon } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";
import { ChatInput } from "./chat-input";

export function EmptyState({
  onSuggestion,
  onSend,
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
          className="mx-auto max-w-[16ch] text-4xl font-medium leading-[1.04] text-[var(--v2-text-strong)] sm:text-5xl lg:text-6xl"
        >
          {t("chat.heroTitle")}
        </h2>
        <p
          className="mx-auto mt-4 max-w-[64ch] text-base leading-relaxed text-[var(--v2-text-muted)]"
        >
          {t("chat.heroDesc")}
        </p>
      </div>

      <div className="mt-9 w-full max-w-5xl">
        <ChatInput
          onSend={onSend}
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
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        {suggestions.map(
          (item) => (
            <button
              type="button"
              key={item.title}
              onClick={() => onSuggestion(item.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-[var(--v2-panel-border)] px-2 py-4 text-left hover:border-[var(--v2-accent)]/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)] group-hover:border-[var(--v2-accent)]/35 group-hover:text-[var(--v2-accent-text)]"
              >
                <Icon name={item.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-medium text-[var(--v2-text-strong)]">
                  {item.title}
                </span>
                <span className="mt-0.5 block text-sm text-[var(--v2-text-muted)]">
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
