import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { CONNECTION_STATUS } from "../lib/connection-status.js";

const STYLES = {
  [CONNECTION_STATUS.CONNECTED]: "bg-mint/20 text-mint border-mint/30",
  [CONNECTION_STATUS.RECONNECTING]: "bg-copper/20 text-copper border-copper/30",
  [CONNECTION_STATUS.DISCONNECTED]: "bg-red-500/20 text-red-200 border-red-400/30",
  [CONNECTION_STATUS.CONNECTING]: "bg-iron-700/50 text-iron-200 border-iron-700/50",
  [CONNECTION_STATUS.PAUSED]: "bg-iron-700/50 text-iron-200 border-iron-700/50",
  [CONNECTION_STATUS.IDLE]: "hidden",
};

const HIDDEN_STATUSES = new Set([
  CONNECTION_STATUS.IDLE,
  CONNECTION_STATUS.CONNECTING,
  CONNECTION_STATUS.CONNECTED,
]);

export function ConnectionStatus({ status }) {
  const t = useT();
  if (!status || HIDDEN_STATUSES.has(status)) return null;

  const labelKey = "connection." + status;
  const label = t(labelKey);

  return html`
    <div
      className=${[
        "sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",
        STYLES[status] || STYLES[CONNECTION_STATUS.CONNECTING],
      ].join(" ")}
    >
      ${label !== labelKey ? label : status}
    </div>
  `;
}
