import { useT } from "../../../lib/i18n";
import { CONNECTION_STATUS } from "../lib/connection-status";

const STATUS_STYLES = {
  [CONNECTION_STATUS.RECONNECTING]: "bg-copper/20 text-copper border-copper/30",
  [CONNECTION_STATUS.DISCONNECTED]: "bg-red-500/20 text-red-200 border-red-400/30",
  [CONNECTION_STATUS.CONNECTING]: "bg-iron-700/50 text-iron-200 border-iron-700/50",
  [CONNECTION_STATUS.PAUSED]: "bg-iron-700/50 text-iron-200 border-iron-700/50",
};

const HIDDEN_STATUSES = new Set([
  CONNECTION_STATUS.IDLE,
  CONNECTION_STATUS.CONNECTED,
]);

export function ConnectionStatus({ status }) {
  const t = useT();
  if (!status || HIDDEN_STATUSES.has(status)) return null;

  const labelKey = "connection." + status;
  const label = t(labelKey);

  return (
    <div
      role="status"
      className={[
        "sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",
        STATUS_STYLES[status] || STATUS_STYLES[CONNECTION_STATUS.CONNECTING],
      ].join(" ")}
    >
      {label !== labelKey ? label : status}
    </div>
  );
}
