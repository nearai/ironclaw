import { useEffect, useRef, useState } from "react";

import { Button, Card, Text } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";
import { useTraceCredits } from "../hooks/useTraceCredits";
import { useAccountTraces } from "../hooks/useAccountTraces";
import { mintAccountLoginLink } from "../lib/settings-api";
import { matchesSearch } from "../lib/settings-search";
import { SettingsSearchEmpty } from "./settings-search-empty";

export function formatCredit(value) {
  return (Number(value) || 0).toFixed(2);
}

function formatSignedCredit(value) {
  const numeric = Number(value) || 0;
  return `${numeric >= 0 ? "+" : ""}${numeric.toFixed(2)}`;
}

export function formatTimestamp(value, t) {
  if (!value) return t("traceCommons.never");
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? t("traceCommons.never") : parsed.toLocaleString();
}

// Open the caller's Trace Commons account in a new tab via a one-time login
// link. Pure/injectable so tests can drive it without a browser: `mint` is the
// API call, `open` is the window.open-shaped opener.
//
// Ordering is load-bearing:
// 1. Open a blank tab SYNCHRONOUSLY (before the async mint) so popup blockers
//    attribute it to the user's click — WITHOUT the `noopener` feature, which
//    would make window.open return null and leave nothing to navigate.
//    Reverse-tabnabbing protection comes from severing `win.opener` manually.
// 2. Short-circuit on a blocked popup BEFORE minting: every mint burns a
//    single-use login URL server-side.
// 3. Only then mint and navigate. The URL exists only in this flow — never
//    logged, never stored.
// 4. Defense in depth: even though the backend pins the minted URL to the
//    trust-anchored issuer origin, refuse to navigate to anything that is
//    not absolute http(s) — the about:blank tab inherits the WebUI origin,
//    so a javascript: URL would execute with WebUI-origin access.
export function isSafeLoginLinkUrl(raw) {
  try {
    const parsed = new URL(raw);
    return parsed.protocol === "https:" || parsed.protocol === "http:";
  } catch {
    return false;
  }
}

export async function openAccountLoginLink({ mint, open }) {
  const win = open("about:blank", "_blank");
  if (!win) {
    return { status: "blocked" };
  }
  win.opener = null;
  try {
    const response = await mint();
    if (!response || response.minted !== true || !response.url) {
      win.close();
      return { status: "unavailable" };
    }
    if (!isSafeLoginLinkUrl(response.url)) {
      win.close();
      return { status: "unavailable" };
    }
    win.location = response.url;
    return { status: "opened" };
  } catch (error) {
    win.close();
    return { status: "error", error };
  }
}

// Decide how the submitted-traces section renders. Pure so the branch logic is
// unit-testable: "error" wins over everything (a tracesQuery failure must
// surface, never silently hide behind an empty state), "list" needs an
// enrolled contributor with at least one trace, anything else renders nothing.
export function tracesSectionMode({ isError, enrolled, traces }) {
  if (isError) return "error";
  if (enrolled && Array.isArray(traces) && traces.length > 0) return "list";
  return "hidden";
}

function StatRow({ label, value, description = "" }) {
  return (
    <div
      className="flex items-center justify-between gap-3 border-t border-[var(--v2-panel-border)] py-3 first:border-0"
    >
      <div className="min-w-0">
        <Text as="div" variant="body" tone="strong">{label}</Text>
        {description &&
        (<Text as="div" variant="caption" tone="muted" className="mt-0.5">{description}</Text>)}
      </div>
      <div className="shrink-0 font-mono text-sm text-[var(--v2-text-strong)]">{value}</div>
    </div>
  );
}

export function TraceCommonsTab({ searchQuery = "" }) {
  const t = useT();
  const { credits, query, authorize } = useTraceCredits();
  const { traces, enrolled: tracesEnrolled, query: tracesQuery } = useAccountTraces();
  const [openState, setOpenState] = useState("idle");
  // Imperative guards: `openInFlightRef` makes double-click protection
  // independent of render timing (each extra click would burn a one-time
  // link), and `mountedRef` prevents a state update landing after unmount.
  const openInFlightRef = useRef(false);
  const mountedRef = useRef(true);
  useEffect(
    () => () => {
      mountedRef.current = false;
    },
    []
  );

  const handleOpenAccount = async () => {
    if (openInFlightRef.current) return;
    openInFlightRef.current = true;
    setOpenState("pending");
    const result = await openAccountLoginLink({
      mint: mintAccountLoginLink,
      open: (url, target) => window.open(url, target),
    });
    openInFlightRef.current = false;
    if (mountedRef.current) {
      setOpenState(result.status === "opened" ? "idle" : "failed");
    }
  };

  if (
    !matchesSearch(searchQuery, [
      "trace commons",
      "credits",
      t("settings.traceCommons"),
      t("traceCommons.title"),
    ])
  ) {
    return (<SettingsSearchEmpty query={searchQuery} />);
  }

  let body;
  if (query.isLoading) {
    body = (
      <div className="mt-4">
        {[1, 2, 3].map(
          (i) => (
            <div
              key={i}
              className="flex items-center justify-between border-t border-[var(--v2-panel-border)] py-3 first:border-0"
            >
              <div className="v2-skeleton h-4 w-32 rounded" />
              <div className="v2-skeleton h-4 w-16 rounded" />
            </div>
          )
        )}
      </div>
    );
  } else if (query.isError) {
    body = (
      <div
        className="mt-4 rounded-xl border border-[color-mix(in_srgb,var(--v2-danger-text)_35%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-4 py-3 text-sm text-[var(--v2-danger-text)]"
      >
        {t("traceCommons.loadFailed")}
      </div>
    );
  } else if (!credits || (!credits.enrolled && !(credits.submissions_total > 0))) {
    body = (
      <div
        className="mt-4 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-6 text-center text-sm text-[var(--v2-text-muted)]"
      >
        {t("traceCommons.emptyState")}
      </div>
    );
  } else {
    const explanations = credits.recent_explanations || [];
    const holds = credits.holds || [];
    body = (
      <>
      <div className="mt-4">
        <StatRow
          label={t("traceCommons.enrollment")}
          value={credits.enrolled ? t("traceCommons.enrolled") : t("traceCommons.notEnrolled")}
        />
        <StatRow
          label={t("traceCommons.pendingCredit")}
          description={t("traceCommons.pendingCreditDesc")}
          value={formatCredit(credits.pending_credit)}
        />
        <StatRow
          label={t("traceCommons.finalCredit")}
          description={t("traceCommons.finalCreditDesc")}
          value={formatCredit(credits.final_credit)}
        />
        <StatRow
          label={t("traceCommons.delayedLedger")}
          description={t("traceCommons.delayedLedgerDesc")}
          value={formatSignedCredit(credits.delayed_credit_delta)}
        />
        <StatRow
          label={t("traceCommons.submissions")}
          value={t("traceCommons.submissionsValue", {
            submitted: credits.submissions_submitted || 0,
            accepted: credits.submissions_accepted || 0,
            total: credits.submissions_total || 0,
          })}
        />
        <StatRow
          label={t("traceCommons.lastSubmission")}
          value={formatTimestamp(credits.last_submission_at, t)}
        />
        <StatRow
          label={t("traceCommons.lastSync")}
          description={t("traceCommons.lastSyncDesc")}
          value={formatTimestamp(credits.last_credit_sync_at, t)}
        />
      </div>
      {explanations.length > 0 &&
      (
        <div className="mt-5">
          <Text as="h4" variant="eyebrow" tone="accent" className="mb-2">
            {t("traceCommons.recentExplanations")}
          </Text>
          <Text as="ul" variant="caption" tone="muted" className="ml-4 list-disc space-y-1">
            {explanations.map((line, index) => (<li key={index}>{line}</li>))}
          </Text>
        </div>
      )}
      {holds.length > 0 &&
      (
        <div className="mt-5">
          <Text as="h4" variant="eyebrow" tone="accent" className="mb-1">
            {t("traceCommons.heldTitle")}
          </Text>
          <Text as="p" variant="caption" tone="muted" className="mb-2">
            {t("traceCommons.heldDescription")}
          </Text>
          <ul className="space-y-2">
            {holds.map(
              (hold) => (
                <li
                  key={hold.submission_id}
                  className="flex items-start justify-between gap-3 rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2"
                >
                  <div className="min-w-0">
                    <Text as="div" variant="caption" tone="strong">{hold.reason}</Text>
                    <div className="mt-0.5 truncate font-mono text-[10px] text-[var(--v2-text-faint)]">
                      {hold.submission_id}
                    </div>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={() => authorize.mutate(hold.submission_id)}
                    disabled={authorize.isPending}
                    className="shrink-0 border-[var(--v2-accent-soft)] font-medium text-[var(--v2-accent-text)] hover:bg-[var(--v2-accent-soft)] hover:text-[var(--v2-accent-text)]"
                  >
                    {authorize.isPending
                      ? t("traceCommons.authorizing")
                      : t("traceCommons.authorize")}
                  </Button>
                </li>
              )
            )}
          </ul>
        </div>
      )}
      </>
    );
  }

  // The submitted-traces section depends only on the traces query — NOT on the
  // credits/empty-state branch above — so it renders independently. Instance-only
  // enrollment is a separate path from personal-invite credits: an instance
  // contributor with no personal credits hits the credits empty-state branch, so
  // gating this on that branch would silently hide their traces and any
  // tracesQuery errors. Render it after `body` regardless of the credits state.
  const tracesMode = tracesSectionMode({
    isError: tracesQuery.isError,
    enrolled: tracesEnrolled,
    traces,
  });
  const tracesSection = tracesMode !== "hidden" && (
      <div className="mt-5">
        <Text as="h4" variant="eyebrow" tone="accent" className="mb-1">
          {t("traceCommons.submittedTracesTitle")}
        </Text>
        {tracesMode === "error" ? (
          <div
            className="rounded-xl border border-[color-mix(in_srgb,var(--v2-danger-text)_35%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-4 py-3 text-sm text-[var(--v2-danger-text)]"
          >
            {t("traceCommons.tracesLoadFailed")}
          </div>
        ) : (
        <ul className="space-y-2">
          {traces.map(
            (trace) => (
              <li
                key={trace.submission_id}
                className="rounded-xl border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2"
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="truncate font-mono text-[10px] text-[var(--v2-text-faint)]">
                    {trace.submission_id}
                  </div>
                  <div
                    className="shrink-0 font-mono text-xs text-[var(--v2-text-strong)]"
                    title={t("traceCommons.traceStatus")}
                    aria-label={t("traceCommons.traceStatus")}
                  >
                    {trace.status}
                  </div>
                </div>
                <div className="mt-1 flex gap-4 text-xs text-[var(--v2-text-muted)]">
                  <span>
                    {t("traceCommons.tracePendingCredit")}:{" "}
                    <span className="font-mono text-[var(--v2-text-strong)]">
                      {formatCredit(trace.pending_credit)}
                    </span>
                  </span>
                  <span>
                    {t("traceCommons.traceFinalCredit")}:{" "}
                    <span className="font-mono text-[var(--v2-text-strong)]">
                      {trace.final_credit != null
                        ? formatCredit(trace.final_credit)
                        : "—"}
                    </span>
                  </span>
                  <span className="ml-auto shrink-0">
                    {t("traceCommons.traceReceivedAt")}:{" "}
                    <span className="font-mono text-[var(--v2-text-strong)]">
                      {formatTimestamp(trace.received_at, t)}
                    </span>
                  </span>
                </div>
              </li>
            )
          )}
        </ul>
        )}
      </div>
  );

  return (
    <Card padding="md">
      <Text as="h3" variant="eyebrow" tone="accent" className="mb-2">
        {t("traceCommons.title")}
      </Text>
      <Text variant="body" tone="muted">
        {t("traceCommons.description")}
      </Text>

      {(credits?.enrolled || tracesEnrolled) && (
        <div className="mt-3">
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={handleOpenAccount}
            disabled={openState === "pending"}
            className="border-[var(--v2-accent-soft)] font-medium text-[var(--v2-accent-text)] hover:bg-[var(--v2-accent-soft)] hover:text-[var(--v2-accent-text)]"
          >
            {openState === "pending"
              ? t("traceCommons.openingAccount")
              : t("traceCommons.openAccount")}
          </Button>
          {openState === "failed" && (
            <Text variant="caption" tone="danger" className="ml-3">
              {t("traceCommons.openAccountFailed")}
            </Text>
          )}
        </div>
      )}

      {body}

      {tracesSection}

      <Text as="p" variant="caption" tone="faint" className="mt-5">
        {t("traceCommons.note")}
      </Text>
    </Card>
  );
}
