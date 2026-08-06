import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { Badge, Panel } from "../../../design-system/primitives";
import React from "react";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";

/**
 * Resolve a Badge tone for a notification-channel row.
 *   "unavailable" → warning (yellow) — a stored channel that no longer
 *                   resolves through the target registry.
 *   anything else → success (green) — a live, selectable channel.
 */
function rowTone(status) {
  if (status === "unavailable") return "warning";
  return "success";
}

export function NotificationChannelsPanel({ channelsState }) {
  const t = useT();
  const rows = channelsState.rows;
  // `selectedIds` is the server truth (the caller's stored notification-
  // channel set, spec §7); `draftIds` is the staged, locally-toggled copy
  // Save posts as a full replace.
  const [draftIds, setDraftIds] = React.useState(
    () => new Set(channelsState.selectedIds),
  );
  const [showSaved, setShowSaved] = React.useState(false);
  const savedTimerRef = React.useRef(null);

  // Resync the draft whenever the stored set changes (initial load, a
  // successful save reconciling from the response, or another tab's write)
  // — matches the single-select predecessor's resync pattern, generalized
  // to a Set. Keyed on a stable string so an equal-but-new array/Set from a
  // fresh query response does not spuriously clobber an in-progress toggle.
  const storedKey = React.useMemo(
    () => [...channelsState.selectedIds].sort().join(" "),
    [channelsState.selectedIds],
  );
  React.useEffect(() => {
    setDraftIds(new Set(channelsState.selectedIds));
  }, [storedKey]);

  React.useEffect(
    () => () => {
      if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
    },
    [],
  );

  const isBusy = channelsState.isLoading || channelsState.isSaving;
  const draftKey = React.useMemo(
    () => [...draftIds].sort().join(" "),
    [draftIds],
  );
  const isDirty = draftKey !== storedKey;
  const canSave = isDirty && !isBusy;

  const toggle = (targetId) => {
    if (isBusy) return;
    setDraftIds((current) => {
      const next = new Set(current);
      if (next.has(targetId)) {
        next.delete(targetId);
      } else {
        next.add(targetId);
      }
      return next;
    });
  };

  // Flash the "Saved" confirmation; the mutation's rejection is reflected
  // through `channelsState.saveError` (rendered below), so the catch here
  // only prevents an unhandled promise rejection. Clear any lingering
  // "Saved" flash up front: the error alert is gated on `!showSaved`, so a
  // stale flash from a prior success would otherwise hide a new failure.
  const flashSavedOnSuccess = (promise) => {
    if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
    setShowSaved(false);
    return promise
      .then(() => {
        if (savedTimerRef.current) clearTimeout(savedTimerRef.current);
        setShowSaved(true);
        savedTimerRef.current = setTimeout(() => setShowSaved(false), 2200);
      })
      .catch(() => {});
  };

  // A stored-but-dead channel renders as a checked "Unavailable" row, but the
  // backend's full-replace write rejects the whole set on the first
  // unresolvable id (`notification_channel_not_found`) — so unresolvable rows
  // are kept rendered but never sent. Saving therefore also drops a dead id
  // from the stored set; the post-save resync unchecks its row.
  const unavailableIds = React.useMemo(
    () =>
      new Set(
        rows
          .filter((row) => row.status === "unavailable")
          .map((row) => row.target_id),
      ),
    [rows],
  );

  const handleSave = () => {
    if (!canSave) return;
    flashSavedOnSuccess(
      channelsState.saveNotificationChannels(
        Array.from(draftIds).filter((id) => !unavailableIds.has(id)),
      ),
    );
  };

  return (
    <Panel className="p-5 sm:p-6">
      <div className="flex flex-col gap-5">

        {/* ── Header ──────────────────────────────────────────────── */}
        <div className="flex flex-col gap-1">
          <div className="font-mono text-[11px] uppercase tracking-[0.16em] text-[var(--v2-text-muted)]">
            {t("automations.notificationChannels.eyebrow")}
          </div>
          <h2 className="mt-1 text-xl font-semibold tracking-[-0.02em] text-[var(--v2-text-strong)]">
            {t("automations.notificationChannels.title")}
          </h2>
          <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
            {t("automations.notificationChannels.explainer")}
          </p>
        </div>

        <hr className="border-t border-[var(--v2-panel-border)]" />

        {/* ── Checkbox rows ────────────────────────────────────────── */}
        <div className="flex flex-col gap-3">
          {rows.map((row) => {
            const isSelected = draftIds.has(row.target_id);
            const isUnavailable = row.status === "unavailable";
            return (
              <label
                key={row.target_id}
                className={cn(
                  "flex items-start gap-3.5 rounded-xl border px-4 py-3.5 cursor-pointer",
                  "transition-colors duration-100",
                  isUnavailable
                    ? "border-dashed bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)] opacity-70"
                    : "bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
                  isSelected &&
                    !isUnavailable &&
                    "border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]",
                )}
              >
                <input
                  type="checkbox"
                  checked={isSelected}
                  disabled={isBusy}
                  onChange={() => toggle(row.target_id)}
                  className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
                />
                <div className="flex-1 min-w-0">
                  <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
                    {row.display_name}
                  </div>
                  {row.description &&
                  (<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                    {row.description}
                  </div>)}
                </div>
                <Badge
                  tone={rowTone(row.status)}
                  label={isUnavailable
                    ? t("automations.notificationChannels.pill.unavailable")
                    : t("automations.notificationChannels.pill.ready")}
                  className="self-center shrink-0"
                />
              </label>
            );
          })}

          {rows.length === 0 &&
          (
            <div
              className="rounded-xl border border-dashed border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3.5 text-sm text-[var(--v2-text-muted)]"
            >
              {t("automations.notificationChannels.empty")}
            </div>
          )}
        </div>

        {/* ── Save row ─────────────────────────────────────────────── */}
        <div className="flex flex-wrap items-center gap-3">
          <Button
            variant="primary"
            size="sm"
            disabled={!canSave}
            onClick={handleSave}
          >
            <Icon name="check" className="h-3.5 w-3.5" />
            {t("automations.notificationChannels.save")}
          </Button>
          {showSaved &&
          (
            <span
              role="status"
              className="flex items-center gap-1.5 text-xs font-semibold text-[var(--v2-positive-text)]"
            >
              <Icon name="check" className="h-3 w-3" />
              {t("automations.notificationChannels.saved")}
            </span>
          )}
          {channelsState.saveError &&
          !showSaved &&
          (
            <span
              role="alert"
              className="flex items-center gap-1.5 text-xs font-semibold text-red-300"
            >
              <Icon name="close" className="h-3 w-3" />
              {t("automations.notificationChannels.saveFailed")}
            </span>
          )}
        </div>

        {/* ── Empty-selection helper (draft, not stored — reflects what
             the next Save would do) ──────────────────────────────── */}
        {draftIds.size === 0 &&
        (
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            {t("automations.notificationChannels.webOnlyHelper")}
          </div>
        )}

      </div>
    </Panel>
  );
}
