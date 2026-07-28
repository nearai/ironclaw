import { useT } from "../../../lib/i18n";

const tone = {
  success: "border-mint/30 bg-mint/10 text-mint",
  error: "border-red-400/30 bg-red-500/10 text-red-200",
  info: "border-signal/30 bg-signal/10 text-signal",
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
