import { useT } from "../../../lib/i18n";

const tone = {
  success: "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
  error: "border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]",
  info: "border-[color-mix(in_srgb,var(--v2-accent-text)_30%,var(--v2-panel-border))] bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]",
};

export function FeedbackBanner({ result, onDismiss }) {
  const t = useT();
  if (!result) return null;

  return (
    <div className={["flex items-center gap-3 rounded-xl border px-4 py-3 text-sm", tone[result.type] || tone.info].join(" ")}>
      <span className="min-w-0 flex-1">{result.message}</span>
      <button onClick={onDismiss} className="shrink-0 opacity-70 hover:opacity-100">{t("projects.feedback.dismiss")}</button>
    </div>
  );
}
