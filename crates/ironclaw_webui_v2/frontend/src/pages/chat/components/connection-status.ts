import { html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";

const STYLES = {
  connected: "bg-mint/20 text-mint border-mint/30",
  reconnecting: "bg-copper/20 text-copper border-copper/30",
  disconnected: "bg-red-500/20 text-red-200 border-red-400/30",
  connecting: "bg-iron-700/50 text-iron-200 border-iron-700/50",
  paused: "bg-iron-700/50 text-iron-200 border-iron-700/50",
  idle: "hidden",
};

export function ConnectionStatus({ status }) {
  const t = useT();
  if (status === "idle" || status === "connecting" || status === "connected" || !status) return null;

  const labelKey = "connection." + status;
  const label = t(labelKey);

  return html`
    <div
      className=${[
        "sticky top-4 z-20 mx-auto mt-4 md:mt-0 mb-2 max-w-md rounded-full border px-4 py-1.5 text-center text-xs font-medium backdrop-blur-xl",
        STYLES[status] || STYLES.connecting,
      ].join(" ")}
    >
      ${label !== labelKey ? label : status}
    </div>
  `;
}
