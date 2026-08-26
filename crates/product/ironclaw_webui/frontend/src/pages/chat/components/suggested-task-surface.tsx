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
 * injection through the composer. Cards stay tool-agnostic: connect remains a
 * separate surface (§3.1), so the drawer offers an *entry* into `/extensions`
 * rather than a per-card connect state.
 *
 * Two controls beyond the shipped set (issue #7815, F1/F2):
 *   refresh → re-runs `generate` on a ready set. The backend is replace-only
 *             (a new `client_action_id` hides the prior generation), so this
 *             is honestly a refresh, not "more"; it becomes additive when the
 *             backend gains a top-up transition.
 *   connect → the first leg of the flow (connect tools -> ask for
 *             suggestions), routed to the existing extensions surface.
 */
import React from "react";
import type { ReactNode } from "react";
import { Link } from "react-router";

import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { useSuggestions } from "../hooks/useSuggestions";
import { SuggestedTaskCard } from "./suggested-task-card";

// The shipped connect surface (extensions catalog + per-extension setup).
const EXTENSIONS_ROUTE = "/extensions";

export function SuggestedTaskSurface({
  onOpenThread,
  renderRunningIndicator,
  hidden = false,
  onClose,
}: {
  onOpenThread?: (threadId: string) => void;
  renderRunningIndicator?: (label: string) => ReactNode;
  // Section-level dismiss: the parent (empty-state) hides the whole drawer and
  // shows a "Show suggestions" pill to restore it. Distinct from per-card
  // dismiss, which removes one suggestion.
  hidden?: boolean;
  onClose?: () => void;
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
  // Section-dismissed: the parent shows the restore pill instead.
  if (hidden) return null;

  const hasCards = suggestions.length > 0;
  // `generating` is the backend's own status; `isGenerating` also covers the
  // moment between the click and the 202 landing.
  const generating = isGenerating || status === "generating";

  // The docked drawer frame (V4) appears once there is something to show — a
  // card set or an in-flight generation. The empty/failed CTA states stay
  // frameless so a lone button isn't wrapped in a big panel.
  const showFrame = hasCards || generating;

  return (
    <section
      aria-label={t("chat.oobe.heading")}
      className="mt-8 w-full max-w-5xl text-left"
    >
      {showFrame ? (
        <div className="rounded-2xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-3 shadow-[var(--v2-card-shadow)]">
          <div className="mb-2.5 flex items-baseline gap-x-2 px-0.5">
            <span className="text-[12px] font-semibold text-[var(--v2-text-strong)]">
              {t("chat.oobe.heading")}
            </span>
            <span className="text-[11px] text-[var(--v2-text-faint)]">
              {t("chat.oobe.subtitle")}
            </span>
            <div className="ml-auto -my-0.5 flex shrink-0 items-center gap-1 self-start">
              <Link
                to={EXTENSIONS_ROUTE}
                aria-label={t("chat.oobe.action.connect")}
                title={t("chat.oobe.action.connect")}
                className="flex h-6 items-center gap-1 rounded-[6px] px-1.5 text-[11px] font-medium text-[var(--v2-text-faint)] transition-colors hover:text-[var(--v2-text-strong)]"
              >
                <Icon name="plug" className="h-3.5 w-3.5" />
                {/* Label drops below `sm`: three controls plus the label wrap
                    the heading onto a second line at 375px. The icon keeps its
                    accessible name via aria-label/title. */}
                <span className="hidden sm:inline">
                  {t("chat.oobe.action.connect")}
                </span>
              </Link>
              <button
                type="button"
                onClick={() => generate()}
                disabled={generating}
                aria-label={t("chat.oobe.action.refresh")}
                title={t("chat.oobe.action.refresh")}
                className="grid h-6 w-6 place-items-center rounded-[6px] text-[var(--v2-text-faint)] transition-colors hover:text-[var(--v2-text-strong)] disabled:cursor-not-allowed disabled:opacity-40"
              >
                <Icon name="retry" className="h-3.5 w-3.5" />
              </button>
              <button
                type="button"
                onClick={() => onClose?.()}
                aria-label={t("chat.oobe.hideSuggestions")}
                className="grid h-6 w-6 place-items-center rounded-[6px] text-[var(--v2-text-faint)] transition-colors hover:text-[var(--v2-text-strong)]"
              >
                <Icon name="close" className="h-4 w-4" />
              </button>
            </div>
          </div>
          {hasCards ? renderCards() : renderSkeleton()}
        </div>
      ) : (
        <div>
          <div className="mb-2 text-[11px] font-medium uppercase tracking-wide text-[var(--v2-text-faint)]">
            {t("chat.oobe.heading")}
          </div>
          {renderCta()}
        </div>
      )}
    </section>
  );

  // Horizontal scrollable strip (matches the mockup): cards are fixed-width and
  // overflow into a scroll region rather than reflowing into a grid.
  function renderCards() {
    return (
      <div className="oobe-strip flex gap-2 overflow-x-auto pb-1">
        {suggestions.map((suggestion) => (
          <div key={suggestion.id} className="w-[248px] shrink-0">
            <SuggestedTaskCard
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
          </div>
        ))}
      </div>
    );
  }

  // Anticipatory beat (V3): the branded working indicator over skeleton tiles,
  // so a generating surface reads as "on it" rather than empty. Tiles use the
  // static `.v2-skeleton` (no shimmer) to respect the motion policy; the NEAR
  // indicator carries the only motion, and it's already a sanctioned exception.
  function renderSkeleton() {
    return (
      <div>
        {renderRunningIndicator
          ? renderRunningIndicator(t("chat.oobe.status.generating"))
          : null}
        <div className="oobe-strip mt-2 flex gap-2 overflow-x-auto pb-1" aria-hidden="true">
          {[0, 1, 2].map((i) => (
            <div
              key={i}
              className="flex w-[248px] shrink-0 flex-col gap-2 rounded-[13px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3"
            >
              <div className="v2-skeleton h-4 w-4/5" />
              <div className="v2-skeleton h-3 w-full" />
              <div className="v2-skeleton mt-2 h-7 w-24" />
            </div>
          ))}
        </div>
      </div>
    );
  }

  // Route entry into the connect surface, for the CTA rows. Rendered after the
  // generate/retry action so generation stays the primary action of the empty
  // state. (The framed drawer carries its own compact header entry.)
  function renderConnectAction() {
    return (
      <Button as={Link} to={EXTENSIONS_ROUTE} variant="ghost" size="sm">
        <Icon name="plug" className="mr-1 h-3.5 w-3.5" />
        {t("chat.oobe.action.connect")}
      </Button>
    );
  }

  function renderCta() {
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
          {renderConnectAction()}
        </div>
      );
    }
    // `empty`, or a `ready` set the user has dismissed down to nothing.
    return (
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="secondary" size="sm" onClick={() => generate()}>
          <Icon name="spark" className="mr-1 h-3.5 w-3.5" />
          {t("chat.oobe.action.generate")}
        </Button>
        {renderConnectAction()}
      </div>
    );
  }
}
