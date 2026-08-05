import React from "react";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { ChatInput } from "./chat-input";
import { AutomationCarousel } from "./automation-carousel";
import { useAutomationTasks } from "../hooks/useAutomationTasks";

export function EmptyState({
  onSuggestion,
  onSend,
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
  const automations = useAutomationTasks();
  const [composerFocused, setComposerFocused] = React.useState(false);
  const showCarousel = !automations.loading && automations.tasks.length > 0;
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

      {showCarousel &&
      (
        // Collapse the "Done for you" strip once the user focuses the composer,
        // freeing vertical room to chat. grid-rows 1fr→0fr animates the height
        // without needing to measure the content.
        <div
          aria-hidden={composerFocused}
          className={[
            "grid w-full max-w-5xl overflow-hidden transition-all duration-300 ease-out",
            composerFocused
              ? "mt-0 grid-rows-[0fr] opacity-0 pointer-events-none"
              : "mt-9 grid-rows-[1fr] opacity-100",
          ].join(" ")}
        >
          <div className="min-h-0 overflow-hidden">
            <AutomationCarousel automations={automations} />
          </div>
        </div>
      )}

      <div className={`${showCarousel && !composerFocused ? "mt-6" : "mt-9"} w-full max-w-5xl transition-all duration-300`}>
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
          onFocusChange={setComposerFocused}
        />
      </div>

      <div className="mt-5 flex w-full max-w-5xl flex-wrap justify-center gap-2">
        {suggestions.map(
          (item) => (
            <button
              type="button"
              key={item.title}
              onClick={() => onSuggestion(item.title)}
              title={item.detail}
              className="v2-button group inline-flex items-center gap-2 rounded-full border border-white/10 bg-white/[0.035] px-3.5 py-2 text-sm text-iron-200 transition-colors hover:border-signal/35 hover:text-white"
            >
              <Icon
                name={item.icon}
                className="h-3.5 w-3.5 shrink-0 text-iron-400 group-hover:text-signal"
              />
              <span>{item.title}</span>
            </button>
          )
        )}
      </div>
    </div>
  );
}
