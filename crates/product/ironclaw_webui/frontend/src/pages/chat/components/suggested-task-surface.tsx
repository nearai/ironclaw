/**
 * SuggestedTaskSurface — the landing surface for backend-generated suggestion
 * cards (VISION-RECONCILIATION §5.1).
 *
 * Data comes from the durable backend contract (PR #7694) via `useSuggestions`:
 * `GET /suggestions` for current state, `POST /suggestions/generate` to ask for
 * a set, `POST /suggestions/{id}/start` to run one, `DELETE /suggestions/{id}`
 * to dismiss. Nothing here is mock data, and the browser never invents card
 * state.
 *
 * Generation status drives the surface (V3 anticipatory states):
 *   empty      → a CTA; generation costs a model run, so the user asks for it
 *   generating → the branded working indicator (the anticipatory beat)
 *   ready      → the cards
 *   failed     → a retry affordance
 *
 * Approve starts the suggestion's own thread/run server-side and reports the
 * returned `thread_id` upward so the app can navigate to it — no prompt
 * injection through the composer. Cards are tool-agnostic: connect is a
 * separate surface (§3.1), so there is no connect state here.
 */
import React from "react";
import type { ReactNode } from "react";

import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { useSuggestions } from "../hooks/useSuggestions";
import { SuggestedTaskCard } from "./suggested-task-card";

export function SuggestedTaskSurface({
  onOpenThread,
  renderRunningIndicator,
}: {
  onOpenThread?: (threadId: string) => void;
  renderRunningIndicator?: (label: string) => ReactNode;
} = {}) {
  const t = useT();
  const {
    isLoading,
    status,
    suggestions,
    generate,
    isGenerating,
    start,
    startingId,
    dismiss,
  } = useSuggestions();

  // Don't render anything until the first read resolves: showing a "generate"
  // CTA over a set that already exists would be a lie, and a flash of empty
  // state on every landing visit is worse than a beat of nothing.
  if (isLoading) return null;

  const hasCards = suggestions.length > 0;
  // `generating` is the backend's own status; `isGenerating` also covers the
  // moment between the click and the 202 landing.
  const generating = isGenerating || status === "generating";

  return (
    <section
      aria-label={t("chat.oobe.heading")}
      className="mt-8 w-full max-w-5xl text-left"
    >
      <div className="mb-2 text-[11px] font-medium uppercase tracking-wide text-[var(--v2-text-faint)]">
        {t("chat.oobe.heading")}
      </div>
      {renderBody()}
    </section>
  );

  function renderBody() {
    // Cards win over any transient status: once a set exists, replacing it
    // with a spinner on regeneration would blank the surface the user is using.
    if (hasCards) {
      return (
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
          {suggestions.map((suggestion) => (
            <SuggestedTaskCard
              key={suggestion.id}
              suggestion={suggestion}
              starting={startingId === suggestion.id}
              renderRunningIndicator={renderRunningIndicator}
              onApprove={() => {
                start(suggestion.id, {
                  onSuccess: (response) => {
                    if (response?.thread_id) onOpenThread?.(response.thread_id);
                  },
                });
              }}
              onOpenThread={() => {
                if (suggestion.thread_id) onOpenThread?.(suggestion.thread_id);
              }}
              onDismiss={() => dismiss(suggestion.id)}
            />
          ))}
        </div>
      );
    }

    if (generating) {
      return (
        <div className="py-1">
          {renderRunningIndicator
            ? renderRunningIndicator(t("chat.oobe.status.generating"))
            : null}
        </div>
      );
    }

    if (status === "failed") {
      return (
        <div className="flex flex-wrap items-center gap-2">
          <span className="inline-flex items-center gap-1 rounded-full border border-[color-mix(in_srgb,var(--v2-danger-text)_45%,transparent)] bg-[var(--v2-danger-soft)] px-2 py-0.5 text-[11px] font-medium text-[var(--v2-danger-text)]">
            <Icon name="alert" className="h-3 w-3" />
            {t("chat.oobe.status.generateFailed")}
          </span>
          <Button variant="secondary" size="sm" onClick={() => generate()}>
            <Icon name="retry" className="mr-1 h-3.5 w-3.5" />
            {t("chat.oobe.action.tryAgain")}
          </Button>
        </div>
      );
    }

    // `empty`, or a `ready` set the user has dismissed down to nothing.
    return (
      <Button variant="secondary" size="sm" onClick={() => generate()}>
        <Icon name="spark" className="mr-1 h-3.5 w-3.5" />
        {t("chat.oobe.action.generate")}
      </Button>
    );
  }
}
