import React from "react";
import hotToast, {
  resolveValue,
  Toaster,
  type Toast,
  type ToastType,
} from "react-hot-toast";
import { Icon } from "../design-system/icons";
import { useT } from "../lib/i18n";

const INFO_TONE = "border-[var(--v2-panel-border)] text-[var(--v2-text)]";
const TONE: Record<ToastType, string> = {
  blank: INFO_TONE,
  custom: INFO_TONE,
  loading: INFO_TONE,
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_32%,var(--v2-panel-border))] text-[var(--v2-positive-text)]",
  error:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] text-[var(--v2-danger-text)]",
};
const ICON: Record<ToastType, string> = {
  blank: "bolt",
  custom: "bolt",
  loading: "bolt",
  success: "check",
  error: "close",
};

export function ToastViewport() {
  const t = useT();

  React.useEffect(() => {
    // The library owns and clears its active expiry timers. Remove its global
    // store entries too so a remounted viewport cannot revive stale toasts.
    return () => hotToast.remove();
  }, []);

  return (
    <Toaster position="bottom-right" gutter={8} containerStyle={{ zIndex: 10000 }}>
      {(item: Toast) => (
        <div
          {...item.ariaProps}
          data-testid="toast"
          className={[
            "pointer-events-auto flex items-center gap-2 rounded-xl border bg-[var(--v2-surface)] px-3.5 py-2.5 text-sm shadow-[0_20px_40px_-20px_rgba(0,0,0,0.7)] transition-opacity",
            item.visible ? "opacity-100" : "opacity-0",
            TONE[item.type],
          ].join(" ")}
        >
          <Icon name={ICON[item.type]} className="h-4 w-4 shrink-0" />
          <span className="min-w-0 flex-1">{resolveValue(item.message, item)}</span>
          <button
            type="button"
            aria-label={t("common.dismiss")}
            title={t("common.dismiss")}
            data-testid="toast-dismiss"
            onClick={() => hotToast.dismiss(item.id)}
            className="-mr-1 grid h-7 w-7 shrink-0 place-items-center rounded-md text-current opacity-70 transition hover:bg-[var(--v2-surface-muted)] hover:opacity-100 focus:opacity-100"
          >
            <Icon name="close" className="h-3.5 w-3.5" />
          </button>
        </div>
      )}
    </Toaster>
  );
}
