import { React, html } from "../../../lib/html.js";
import { Icon } from "../../../design-system/icons.js";

const toneCss = {
  success: "border-mint/30 bg-mint/10 text-mint",
  error: "border-red-400/30 bg-red-500/10 text-red-200",
  info: "border-signal/30 bg-signal/10 text-signal",
};

export function ActionToast({ result, onDismiss }) {
  React.useEffect(() => {
    if (!result) return;
    const timer = setTimeout(onDismiss, 4000);
    return () => clearTimeout(timer);
  }, [result, onDismiss]);

  if (!result) return null;

  return html`
    <div className=${[
      "flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",
      toneCss[result.type] || toneCss.info,
    ].join(" ")}>
      <${Icon}
        name=${result.type === "success" ? "check" : result.type === "error" ? "close" : "bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">${result.message}</span>
      <button onClick=${onDismiss} className="shrink-0 opacity-70 hover:opacity-100">
        <${Icon} name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  `;
}
