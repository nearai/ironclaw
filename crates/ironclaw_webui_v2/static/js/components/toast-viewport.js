import { React, html } from "../lib/html.js";
import { subscribeToasts } from "../lib/toast.js";
import { Icon } from "../design-system/icons.js";

const TONE = {
  info: "border-[var(--v2-panel-border)] text-[var(--v2-text)]",
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",
  error:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]",
};
const ICON = { info: "bolt", success: "check", error: "close" };

export function ToastViewport() {
  const [items, setItems] = React.useState([]);

  React.useEffect(
    () =>
      subscribeToasts((item) => {
        setItems((prev) => [...prev, item]);
        setTimeout(
          () => setItems((prev) => prev.filter((t) => t.id !== item.id)),
          item.duration
        );
      }),
    []
  );

  if (items.length === 0) return null;

  return html`
    <div className="pointer-events-none fixed bottom-4 right-4 z-[60] flex flex-col gap-2">
      ${items.map(
        (item) => html`
          <div
            key=${item.id}
            role="status"
            className=${[
              "pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)]",
              TONE[item.tone] || TONE.info,
            ].join(" ")}
          >
            <${Icon} name=${ICON[item.tone] || "bolt"} className="h-4 w-4 shrink-0" />
            <span>${item.message}</span>
          </div>
        `
      )}
    </div>
  `;
}
