import { useT } from "../../../lib/i18n";
import { CONNECTION_STATUS } from "../lib/connection-status";

const FALLBACK_STYLE = "bg-iron-700/50 text-iron-200 border-iron-700/50";

const HIDDEN_STATUSES = new Set([
  CONNECTION_STATUS.IDLE,
  CONNECTION_STATUS.CONNECTING,
  CONNECTION_STATUS.CONNECTED,
  CONNECTION_STATUS.RECONNECTING,
  CONNECTION_STATUS.DISCONNECTED,
  CONNECTION_STATUS.PAUSED,
]);

export function ConnectionStatus({ status }) {
  const t = useT();
  if (!status || HIDDEN_STATUSES.has(status)) return null;

  const labelKey = "connection." + status;
  const label = t(labelKey);

  return (
    <div
      className={[
        "sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",
        FALLBACK_STYLE,
      ].join(" ")}
    >
      {label !== labelKey ? label : status}
    </div>
  );
}
