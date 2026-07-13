import React from "react";
import { useT } from "../../../lib/i18n";
import { CONNECTION_STATUS, type ConnectionStatus } from "../lib/connection-status";

type ConnectionStatusProps = {
  status?: ConnectionStatus | null;
};

const STATUS_STYLES: Partial<Record<ConnectionStatus, string>> = {
  [CONNECTION_STATUS.RECONNECTING]:
    "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]",
  [CONNECTION_STATUS.DISCONNECTED]:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
  [CONNECTION_STATUS.PAUSED]:
    "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
};

const DEFAULT_STATUS_STYLE =
  "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]";

const STATUS_DESKTOP_STYLES: Partial<Record<ConnectionStatus, string>> = {
  [CONNECTION_STATUS.RECONNECTING]:
    "sm:border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] sm:bg-[var(--v2-warning-soft)] sm:text-[var(--v2-warning-text)]",
  [CONNECTION_STATUS.DISCONNECTED]:
    "sm:border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] sm:bg-[var(--v2-danger-soft)] sm:text-[var(--v2-danger-text)]",
  [CONNECTION_STATUS.PAUSED]:
    "sm:border-[var(--v2-panel-border)] sm:bg-[var(--v2-surface-soft)] sm:text-[var(--v2-text-muted)]",
};

const DEFAULT_DESKTOP_STATUS_STYLE =
  "sm:border-[var(--v2-panel-border)] sm:bg-[var(--v2-surface-soft)] sm:text-[var(--v2-text-muted)]";

const STATUS_DOT_STYLES: Partial<Record<ConnectionStatus, string>> = {
  [CONNECTION_STATUS.RECONNECTING]: "text-[var(--v2-warning-text)]",
  [CONNECTION_STATUS.DISCONNECTED]: "text-[var(--v2-danger-text)]",
  [CONNECTION_STATUS.PAUSED]: "text-[var(--v2-text-muted)]",
};

const DEFAULT_DOT_STYLE = "text-[var(--v2-text-muted)]";

const HIDDEN_STATUSES: ReadonlySet<ConnectionStatus> = new Set([
  CONNECTION_STATUS.IDLE,
  CONNECTION_STATUS.CONNECTING,
  CONNECTION_STATUS.CONNECTED,
]);

export function ConnectionStatus({ status }: ConnectionStatusProps) {
  const t = useT();
  const [expanded, setExpanded] = React.useState(false);

  React.useEffect(() => {
    setExpanded(false);
  }, [status]);

  if (!status || HIDDEN_STATUSES.has(status)) return null;

  const labelKey = "connection." + status;
  const label = t(labelKey);
  const statusLabel = label !== labelKey ? label : status;
  const statusStyle = STATUS_STYLES[status] || DEFAULT_STATUS_STYLE;
  const desktopStatusStyle =
    STATUS_DESKTOP_STYLES[status] || DEFAULT_DESKTOP_STATUS_STYLE;
  const dotStyle = STATUS_DOT_STYLES[status] || DEFAULT_DOT_STYLE;

  return (
    <div className="relative shrink-0">
      <span role="status" className="sr-only">{statusLabel}</span>
      <button
        type="button"
        data-testid="connection-status"
        aria-label={statusLabel}
        title={statusLabel}
        onClick={() => setExpanded((current) => !current)}
        className={[
          "inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-transparent bg-transparent p-0 shadow-none transition-opacity hover:opacity-80 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-current sm:h-7 sm:w-auto sm:max-w-48 sm:gap-1.5 sm:px-2.5 sm:text-xs sm:font-medium sm:opacity-100 sm:shadow-[0_8px_20px_-14px_rgba(0,0,0,0.72)]",
          dotStyle,
          desktopStatusStyle,
        ].join(" ")}
      >
        <span
          aria-hidden="true"
          className={[
            "h-1.5 w-1.5 shrink-0 rounded-full bg-current",
            status === CONNECTION_STATUS.RECONNECTING
              ? "animate-[v2-breathe_1.6s_ease-in-out_infinite]"
              : "",
          ].join(" ")}
        />
        <span
          className="hidden max-w-40 truncate sm:block"
        >
          {statusLabel}
        </span>
      </button>
      <div
        data-testid="connection-status-label"
        className={[
          "pointer-events-none absolute right-0 top-[calc(100%+0.375rem)] z-50 w-max max-w-[calc(100vw_-_1.5rem)] rounded-lg border px-3 py-2 text-xs font-medium shadow-[0_12px_28px_-14px_rgba(0,0,0,0.72)] transition duration-150 sm:hidden",
          expanded
            ? "visible translate-y-0 opacity-100"
            : "invisible -translate-y-1 opacity-0",
          statusStyle,
        ].join(" ")}
      >
        {statusLabel}
      </div>
    </div>
  );
}
