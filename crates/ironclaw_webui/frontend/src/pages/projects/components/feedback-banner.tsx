import { useT } from "../../../lib/i18n";

// Kept as main's hand-rolled banner (not the design-system <Callout>): each
// tone here renders main's exact legacy colors (mint/signal aliases and the
// red-* literals), which Callout's token tones intentionally do not reproduce.
// This extraction must not change rendered output; fold into Callout when a
// restyle is actually on the table.
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
      <button onClick={onDismiss} className="shrink-0 rounded opacity-70 hover:opacity-100 focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-focus-ring)]">{t("projects.feedback.dismiss")}</button>
    </div>
  );
}
