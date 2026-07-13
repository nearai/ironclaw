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

  return (
    <div className="shrink-0">
      <span role="status" className="sr-only">{statusLabel}</span>
      <button
        type="button"
        data-testid="connection-status"
        aria-label={statusLabel}
        title={statusLabel}
        onClick={() => setExpanded((current) => !current)}
        className={[
          "inline-flex h-8 shrink-0 items-center gap-1.5 rounded-lg border text-xs font-medium shadow-[0_8px_20px_-14px_rgba(0,0,0,0.72)] transition-[width,padding] duration-150 sm:h-7 sm:w-auto sm:max-w-48 sm:justify-start sm:px-2.5",
          expanded
            ? "max-w-32 justify-start px-2.5"
            : "w-8 justify-center px-0",
          statusStyle,
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
          data-testid="connection-status-label"
          className={[
            expanded ? "max-w-24 truncate" : "hidden",
            "sm:block sm:max-w-40 sm:truncate",
          ].join(" ")}
        >
          {statusLabel}
        </span>
      </button>
    </div>
  );
}
