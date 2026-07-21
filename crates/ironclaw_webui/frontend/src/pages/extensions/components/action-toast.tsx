import React from "react";
import { Icon } from "@ironclaw/design-system";
import { useT } from "../../../lib/i18n";

const toneCss = {
  success:
    "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  error:
    "border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
  info:
    "border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
};

export function ActionToast({ result, onDismiss }) {
  const t = useT();
  React.useEffect(() => {
    if (!result) return;
    const timer = setTimeout(onDismiss, 4000);
    return () => clearTimeout(timer);
  }, [result, onDismiss]);

  if (!result) return null;

  return (
    <div className={[
      "flex items-center gap-3 rounded-xl border px-4 py-3 text-sm",
      toneCss[result.type] || toneCss.info,
    ].join(" ")}>
      <Icon
        name={result.type === "success" ? "check" : result.type === "error" ? "close" : "bolt"}
        className="h-4 w-4 shrink-0"
      />
      <span className="min-w-0 flex-1">{result.message}</span>
      <button
        onClick={onDismiss}
        aria-label={t("common.dismiss")}
        className="shrink-0 opacity-70 hover:opacity-100"
      >
        <Icon name="close" className="h-3.5 w-3.5" />
      </button>
    </div>
  );
}
