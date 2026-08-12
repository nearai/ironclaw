import { Button } from "../../../design-system/button";
import { Icon } from "../../../design-system/icons";
import { Badge, Panel } from "../../../design-system/primitives";
import React from "react";
import { useT } from "../../../lib/i18n";
import { cn } from "../../../utils/cn";
import { getSessionChannelExtensionId } from "../../../lib/api";
import { useDevicePush } from "../hooks/useDevicePush";

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

/**
 * The per-browser device state rendered under the session channel's row.
 * Account-level routing (the checkbox) and device enrollment (this block)
 * are deliberately separate dimensions: selecting the channel routes
 * notifications to the account's enrolled browsers, and this block manages
 * whether THIS browser is one of them.
 */
function DevicePushBlock({ device, t }) {
  const state = device.browser?.state || "checking";
  const hasStatusError = Boolean(device.statusError);
  let stateCopy;
  if (state === "unsupported") {
    stateCopy = t("automations.notificationChannels.devicePush.unsupported");
  } else if (state === "permission-denied") {
    stateCopy = t("automations.notificationChannels.devicePush.permissionDenied");
  } else if (state === "enrolled" || state === "enrolled-unverified") {
    // "enrolled-unverified" (correlation unavailable — e.g. the status query
    // failed) renders the enrolled copy but exposes no disable action; the
    // status-error line below explains why the panel knows less than usual.
    stateCopy = t("automations.notificationChannels.devicePush.enrolled");
  } else if (state === "enrolled-other-account") {
    stateCopy = t("automations.notificationChannels.devicePush.enrolledOtherAccount");
  } else if (state === "not-enrolled") {
    stateCopy = t("automations.notificationChannels.devicePush.notEnrolled");
  } else {
    stateCopy = t("automations.notificationChannels.devicePush.checking");
  }
  // The enroll flow also serves "enable for this account" when another
  // account's subscription occupies this browser: the same backend register
  // reuses the existing subscription without touching the other enrollment.
  const canEnroll =
    (state === "not-enrolled" || state === "enrolled-other-account") &&
    !device.isBusy &&
    device.vapidPublicKey;
  // Local unsubscribe is offered ONLY for a verified own-account enrollment:
  // the push subscription is browser-global, so disabling from any other
  // state could sever a different account's notifications.
  const canUnenroll = state === "enrolled" && !device.isBusy;
  return (
    <div
      className="rounded-[10px] border border-dashed border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-muted)]"
    >
      <div className="font-semibold text-[var(--v2-text-strong)]">
        {t("automations.notificationChannels.devicePush.deviceHeading")}
      </div>
      <div className="mt-1">{stateCopy}</div>
      {hasStatusError
        ? (
          <div role="alert" className="mt-1 text-red-300">
            {t("automations.notificationChannels.devicePush.statusFailed")}
          </div>
        )
        : (
          <div className="mt-1 text-[var(--v2-text-faint)]">
            {t("automations.notificationChannels.devicePush.deviceCount", {
              count: device.subscriptionCount,
            })}
          </div>
        )}
      {(state === "not-enrolled" || state === "enrolled" || state === "enrolled-other-account") &&
      (
        <div className="mt-2 flex items-center gap-3">
          {(state === "not-enrolled" || state === "enrolled-other-account") &&
          (
            <Button
              variant="secondary"
              size="sm"
              disabled={!canEnroll}
              onClick={() => device.enroll().catch(() => {})}
            >
              {state === "enrolled-other-account"
                ? t("automations.notificationChannels.devicePush.enableForAccount")
                : t("automations.notificationChannels.devicePush.enroll")}
            </Button>
          )}
          {state === "enrolled" &&
          (
            <Button
              variant="secondary"
              size="sm"
              disabled={!canUnenroll}
              onClick={() => device.unenroll().catch(() => {})}
            >
              {t("automations.notificationChannels.devicePush.unenroll")}
            </Button>
          )}
        </div>
      )}
      {device.actionError &&
      (
        <div role="alert" className="mt-2 text-red-300">
          {t("automations.notificationChannels.devicePush.actionFailed")}
        </div>
      )}
    </div>
  );
}

/**
 * The label markup every channel row shares: checkbox, name/description, and
 * the status pill. Deliberately a plain function, not a component — called
 * inline it renders its checkbox directly into the panel, which is what the
 * generic rows need. The session channel's row wraps it (see
 * `SessionChannelRow`) to add the device hook and enrollment gating.
 */
function renderChannelRowLabel({
  targetId,
  displayName,
  description,
  isSelected,
  disabled,
  muted,
  badgeTone,
  badgeLabel,
  onToggle,
}) {
  return (
    <label
      className={cn(
        "flex items-start gap-3.5 rounded-xl border px-4 py-3.5",
        disabled ? "cursor-default" : "cursor-pointer",
        "transition-colors duration-100",
        muted
          ? "border-dashed bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)] opacity-70"
          : "bg-[var(--v2-surface-soft)] border-[var(--v2-panel-border)] hover:bg-[var(--v2-surface-muted)] hover:border-[color-mix(in_srgb,var(--v2-accent)_30%,var(--v2-panel-border))]",
        isSelected &&
          !muted &&
          "border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)]",
      )}
    >
      <input
        type="checkbox"
        checked={isSelected}
        disabled={disabled}
        onChange={() => onToggle(targetId)}
        className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--v2-accent)]"
      />
      <div className="flex-1 min-w-0">
        <div className="text-sm font-semibold text-[var(--v2-text-strong)] leading-snug">
          {displayName}
        </div>
        {description &&
        (<div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
          {description}
        </div>)}
      </div>
      <Badge tone={badgeTone} label={badgeLabel} className="self-center shrink-0" />
    </label>
  );
}

/**
 * The session channel's row — the channel this SPA itself fronts, matched by
 * the `GET /session`-advertised extension id (never a channel name). Owns
 * the device-push hook so the account setup-status query mounts ONLY when
 * this row is present (not at the panel top, where it would fire for every
 * automations view, including deployments with no session channel), and
 * gates the row on the channel's generic setup status: not set up means
 * there is nowhere to deliver, so the channel cannot be newly SELECTED — the
 * checkbox is disabled and the pill drops from "Ready". An already-stored
 * selection stays deselectable (disabled only when unchecked), so a browser
 * that unsubscribes never leaves a locked-on checkbox. The device block
 * underneath manages whether THIS browser is one of the enrolled ones.
 */
function SessionChannelRow({ row, isSelected, isEditingLocked, onToggle, t }) {
  const device = useDevicePush({ extensionId: row.channel });
  const isPending = device.isStatusLoading;
  const enrolled = device.subscriptionCount > 0;
  return (
    <div className="flex flex-col gap-2">
      {renderChannelRowLabel({
        targetId: row.target_id,
        displayName: row.display_name,
        description: row.description,
        isSelected,
        disabled: isEditingLocked || (!isSelected && (isPending || !enrolled)),
        muted: !isPending && !enrolled,
        badgeTone: isPending ? "neutral" : enrolled ? "success" : "warning",
        badgeLabel: isPending
          ? t("automations.notificationChannels.devicePush.checking")
          : enrolled
            ? t("automations.notificationChannels.pill.ready")
            : t("automations.notificationChannels.pill.unavailable"),
        onToggle,
      })}
      <DevicePushBlock device={device} t={t} />
    </div>
  );
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

  // A failed read is not a truthful empty set. When the notification-channels
  // GET fails (while the targets GET succeeds) the hook can only yield
  // channels=[] => selectedIds=[], so every stored channel renders UNCHECKED.
  // Toggling one row from that state would stage a dirty draft, and Save
  // posts a FULL REPLACE — silently dropping every stored channel the user
  // never saw. Editing therefore stays locked until the read actually
  // succeeded: the tool-evidence UI rule wants backend evidence or an
  // explicit disabled state, never a guess the user can act on.
  const hasLoadError = Boolean(channelsState.error);
  const isEditingLocked =
    channelsState.isLoading || channelsState.isSaving || hasLoadError;
  const draftKey = React.useMemo(
    () => [...draftIds].sort().join(" "),
    [draftIds],
  );
  const isDirty = draftKey !== storedKey;
  const canSave = isDirty && !isEditingLocked;

  const toggle = (targetId) => {
    if (isEditingLocked) return;
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
          {hasLoadError &&
          (
            <div
              role="alert"
              className="rounded-xl border border-red-400/30 bg-red-500/10 px-4 py-3.5 text-sm text-red-200"
            >
              <div>{t("automations.error.loadFailed")}</div>
              <div className="mt-1 text-xs leading-5 text-red-200/80">
                {t("automations.notificationChannels.loadFailedEditingDisabled")}
              </div>
            </div>
          )}
          {rows.map((row) => {
            const isSelected = draftIds.has(row.target_id);
            // The session channel's row owns its own hook + setup gating;
            // every other channel renders the shared label directly (its
            // checkbox must stay inline in the panel, not behind a child
            // component).
            const sessionChannel = getSessionChannelExtensionId();
            if (sessionChannel && row.channel === sessionChannel) {
              return (
                <SessionChannelRow
                  key={row.target_id}
                  row={row}
                  isSelected={isSelected}
                  isEditingLocked={isEditingLocked}
                  onToggle={toggle}
                  t={t}
                />
              );
            }
            const isUnavailable = row.status === "unavailable";
            return (
              <div key={row.target_id} className="flex flex-col gap-2">
                {renderChannelRowLabel({
                  targetId: row.target_id,
                  displayName: row.display_name,
                  description: row.description,
                  isSelected,
                  disabled: isEditingLocked,
                  muted: isUnavailable,
                  badgeTone: rowTone(row.status),
                  badgeLabel: isUnavailable
                    ? t("automations.notificationChannels.pill.unavailable")
                    : t("automations.notificationChannels.pill.ready"),
                  onToggle: toggle,
                })}
              </div>
            );
          })}

          {!hasLoadError && rows.length === 0 &&
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
             the next Save would do). Suppressed after a failed read: the
             draft is then only an artifact of the channels the GET never
             returned, so asserting that nothing is selected would be a
             claim about state the panel does not actually know. ──────── */}
        {!hasLoadError && draftIds.size === 0 &&
        (
          <div
            className="rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-3 text-xs leading-relaxed text-[var(--v2-text-faint)]"
          >
            {t("automations.notificationChannels.noSelectionHelper")}
          </div>
        )}

      </div>
    </Panel>
  );
}
