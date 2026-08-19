/**
 * SuggestedTaskCard — one backend suggestion rendered as a startable card.
 * Pure and presentational: it takes a `Suggestion` plus callbacks and renders
 * exactly one action row. No data fetching, no auth, no connect state.
 *
 *   not started  → Approve   (onApprove)   — starts the bound thread/run
 *   starting     → live NEAR "working" indicator (the start call is in flight)
 *   started      → View in thread (onOpenThread) — the card keeps its durable
 *                  `thread_id` binding, so a returning user can rejoin the run
 *
 * The card shows the suggestion's semantic `icon` and a "From <sources>"
 * provenance line (both from the backend card schema). Connect is still a
 * separate landing surface, not a card state — see
 * docs/internal/design/oobe/VISION-RECONCILIATION.md §3.1.
 *
 * Live run status (running/completed/failed derived from the bound `run_id`)
 * is a later slice; until it lands the card states nothing it cannot prove.
 */
import type { ReactNode } from "react";

import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { useT } from "../../../lib/i18n";
import { resolveIconId, SuggestionIcon } from "../lib/suggestion-icons";
import { formatSources, type Suggestion } from "../lib/suggestions-api";

export function SuggestedTaskCard({
  suggestion,
  onApprove,
  onOpenThread,
  onDismiss,
  starting = false,
  renderRunningIndicator,
}: {
  suggestion: Suggestion;
  onApprove?: () => void;
  onOpenThread?: () => void;
  onDismiss?: () => void;
  starting?: boolean;
  renderRunningIndicator?: (label: string) => ReactNode;
}) {
  const t = useT();
  const started = Boolean(suggestion.thread_id);
  const iconId = resolveIconId(suggestion);
  const provenance = formatSources(suggestion.sources);

  return (
    <div
      role="group"
      aria-label={suggestion.title}
      className="oobe-card-reveal flex h-full w-full flex-col rounded-[13px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3 text-left transition-colors hover:border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]"
    >
      {/* Task icon + dismiss */}
      <div className="mb-1.5 flex items-center gap-1.5">
        <SuggestionIcon
          id={iconId}
          className="h-5 w-5 shrink-0"
        />
        <button
          type="button"
          onClick={() => onDismiss?.()}
          disabled={starting}
          aria-label={t("chat.oobe.dismiss")}
          className="ml-auto grid h-5 w-5 shrink-0 place-items-center rounded-[6px] text-[var(--v2-text-faint)] transition-colors hover:text-[var(--v2-text-strong)] disabled:cursor-not-allowed"
        >
          <Icon name="close" className="h-3.5 w-3.5" />
        </button>
      </div>

      {/* Title + description */}
      <div className="text-[13px] font-semibold leading-tight text-[var(--v2-text-strong)]">
        {suggestion.title}
      </div>
      <p className="mt-0.5 line-clamp-2 text-[11px] leading-4 text-[var(--v2-text-muted)]">
        {suggestion.description}
      </p>

      {/* Provenance — the tool(s) this suggestion draws on (backend `sources`) */}
      {provenance && (
        <div className="mt-1.5 text-[10.5px] text-[var(--v2-text-faint)]">
          {t("chat.oobe.from", { sources: provenance })}
        </div>
      )}

      {/* One action row */}
      <div className="mt-2.5">{renderActions()}</div>
    </div>
  );

  function renderActions() {
    if (starting) {
      return renderRunningIndicator
        ? renderRunningIndicator(t("chat.oobe.status.starting"))
        : null;
    }
    if (started) {
      return (
        <Button variant="secondary" size="sm" onClick={() => onOpenThread?.()}>
          <Icon name="chat" className="mr-1 h-3.5 w-3.5" />
          {t("chat.oobe.action.openThread")}
        </Button>
      );
    }
    return (
      <Button variant="primary" size="sm" onClick={() => onApprove?.()}>
        <Icon name="check" className="mr-1 h-3.5 w-3.5" />
        {t("chat.oobe.action.approve")}
      </Button>
    );
  }
}
