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
  if (!status || HIDDEN_STATUSES.has(status)) return null;

  const labelKey = "connection." + status;
  const label = t(labelKey);

  return (
    <div
      role="status"
      className={[
        "pointer-events-none absolute right-3 top-3 z-20 inline-flex w-max max-w-[calc(100%_-_1.5rem)] items-center gap-2 rounded-lg border px-3 py-2 text-left text-xs font-medium leading-4 shadow-[0_12px_28px_-14px_rgba(0,0,0,0.72)] backdrop-blur-xl sm:right-4 sm:top-4 sm:max-w-sm",
        STATUS_STYLES[status] || DEFAULT_STATUS_STYLE,
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
      <span>{label !== labelKey ? label : status}</span>
    </div>
  );
}
