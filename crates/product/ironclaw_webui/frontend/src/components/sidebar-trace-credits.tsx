import { Link } from "react-router";
import { Icon } from "../design-system/icons";
import { useT } from "../lib/i18n";
import { useTraceCredits } from "../pages/settings/hooks/useTraceCredits";

function formatSignedCredit(value) {
  const numeric = Number(value) || 0;
  return `${numeric >= 0 ? "+" : ""}${numeric.toFixed(2)}`;
}

// Compact Trace Commons credits summary pinned above the conversation list.
//
// Renders ONLY when the caller is enrolled; the loading, error, and
// not-enrolled states all render nothing so the sidebar stays clean for the
// common case (most installs are not enrolled). Onboarding lives in the agent
// flow and Settings, not here.
//
// This is a glanceable summary, not a second ledger view: it shares the
// `["trace-credits"]` react-query cache with the Settings -> Trace Commons tab
// and clicks through to that tab, which owns the full breakdown.
export function SidebarTraceCredits() {
  const t = useT();
  const { credits } = useTraceCredits();

  if (!credits || !credits.enrolled) return null;

  const final = formatSignedCredit(credits.final_credit);
  const accepted = credits.submissions_accepted || 0;
  const submitted = credits.submissions_submitted || 0;
  const heldCount = credits.manual_review_hold_count || 0;

  return (
    <div className="px-3 pb-1">
      <Link
        to="/settings/traces"
        className="block rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2.5 transition-colors hover:border-[var(--v2-accent-soft)] hover:bg-[var(--v2-surface-muted)]"
      >
        <div className="flex items-center gap-2 text-[var(--v2-accent-text)]">
          <Icon name="layers" className="h-3.5 w-3.5 shrink-0" />
          <span className="min-w-0 truncate font-mono text-[11px] uppercase tracking-[0.14em]">
            {t("settings.traceCommons")}
          </span>
        </div>
        <div className="mt-2 flex items-center justify-between gap-2">
          <span className="text-xs text-[var(--v2-text-muted)]">{t("traceCommons.finalCredit")}</span>
          <span className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">{final}</span>
        </div>
        <div className="mt-0.5 text-[11px] text-[var(--v2-text-muted)]">
          {t("traceCommons.cardAccepted", { accepted, submitted })}
        </div>
        {heldCount > 0 &&
        (
          <div className="mt-1 text-[11px] font-medium text-[var(--v2-accent-text)]">
            {t("traceCommons.cardHeld", { count: heldCount })}
          </div>
        )}
      </Link>
    </div>
  );
}
